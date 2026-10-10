//! What the Import and Export sheets do, as functions of a store and a path.
//!
//! The work itself is [`crate::import`] and [`crate::export`], the same functions `mailo import`
//! and `mailo export` run. This is what a front end asks while a person is still choosing: what a
//! typed path holds ([`look`]), where imported mail can go ([`upload_folders`]), what an export
//! would name its file ([`suggested`]) and how many messages a search means ([`selection_size`]).
//! Every answer is a value; the sheet words it.

use crate::error::CoreError;
use crate::export::{self, Target};
use crate::import::{self, Destination, Imported, Source};
use chrono::{DateTime, Utc};
use mail_domain::Incoming;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// A typed path, with a leading `~` meaning `home`. Surrounding space is not part of a name
/// anybody types on purpose, so it is dropped.
pub fn expand(typed: &str, home: Option<&OsStr>) -> PathBuf {
    let typed = typed.trim();
    match (typed, home) {
        ("~", Some(home)) => PathBuf::from(home),
        // `~/`, and on Windows `~\` too: whatever this platform takes as a separator.
        (_, Some(home))
            if typed.starts_with('~') && typed[1..].starts_with(std::path::is_separator) =>
        {
            PathBuf::from(home).join(&typed[2..])
        }
        _ => PathBuf::from(typed),
    }
}

/// [`expand`] against this user's home directory.
pub fn expand_here(typed: &str) -> PathBuf {
    expand(
        typed,
        crate::config::home_dir().as_deref().map(Path::as_os_str),
    )
}

/// Why the operating system would not do something with a path: its kind, and its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoCause {
    pub kind: ErrorKind,
    pub text: String,
}

impl IoCause {
    fn of(error: &std::io::Error) -> Self {
        Self {
            kind: error.kind(),
            text: error.to_string(),
        }
    }
}

/// Why a typed path cannot be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// There is nothing at the path.
    NothingThere,
    /// Something is there, and it cannot be looked at.
    CannotReach(IoCause),
    /// A file that cannot be opened.
    CannotOpen(IoCause),
    /// A file with nothing in it.
    Empty,
    /// It is not mail, or not mail that can be read: [`import`]'s own words.
    Unreadable(String),
}

/// What a typed path holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// Nothing typed yet.
    Blank,
    /// Mail that can be imported: which kind, and how many messages.
    Mail {
        source: Source,
        count: usize,
    },
    Refused(Refusal),
}

/// Look at `path` the way an import will read it, and count what is there.
///
/// Blocking and as slow as reading the source: an mbox is read to its end to count it.
pub fn look(path: &Path) -> Seen {
    if path.as_os_str().is_empty() {
        return Seen::Blank;
    }
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Seen::Refused(Refusal::NothingThere),
        Err(e) => return Seen::Refused(Refusal::CannotReach(IoCause::of(&e))),
    };
    if meta.is_file() {
        if let Err(e) = std::fs::File::open(path) {
            return Seen::Refused(Refusal::CannotOpen(IoCause::of(&e)));
        }
        if meta.len() == 0 {
            return Seen::Refused(Refusal::Empty);
        }
    }
    let source = match import::detect(path) {
        Ok(source) => source,
        Err(why) => return Seen::Refused(Refusal::Unreadable(why.to_string())),
    };
    match import::items(&source) {
        Ok(items) => Seen::Mail {
            count: items.count(),
            source,
        },
        Err(why) => Seen::Refused(Refusal::Unreadable(why.to_string())),
    }
}

/// An IMAP account's folder that imported mail can be uploaded into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub account: AccountId,
    pub address: String,
    pub folder: String,
}

/// Every folder imported mail can be uploaded into: each IMAP account's folders as the store lists
/// them, accounts oldest first. POP3 has no folders to upload into, and local folders are a place
/// to import into, not an account to upload to. A plan that does not read is taken for IMAP, as
/// the window takes it, so its tile still has folders to name.
pub fn upload_folders(store: &SqliteStore) -> Vec<Upload> {
    let mut out = Vec::new();
    for account in store.list_accounts().unwrap_or_default() {
        let imap = match &account.plan {
            Ok(plan) => matches!(plan.incoming, Incoming::Imap { .. }),
            Err(_) => true,
        };
        if !imap {
            continue;
        }
        let mut folders: Vec<String> = store
            .folders(account.id.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|folder| folder.path)
            .collect();
        folders.sort();
        out.extend(folders.into_iter().map(|folder| Upload {
            account: account.id.clone(),
            address: account.address.clone(),
            folder,
        }));
    }
    out
}

/// Import `source` into `into`: kept in local folders, or queued for a folder's upload. Returns
/// what was imported, and the account whose outbox now holds uploads, if any.
///
/// Nothing leaves here; sending the uploads is a sync's.
pub fn import_into(
    store: &SqliteStore,
    source: &Source,
    into: &Destination,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<(Imported, Option<AccountId>), CoreError> {
    match into {
        Destination::Local => Ok((import::into_local(store, source, now, progress)?, None)),
        Destination::Mailbox { account, folder } => {
            let (queued, total) =
                import::queue_uploads(store, account, folder, source, now, progress)?;
            Ok((total, Some(queued)))
        }
    }
}

/// The three ways an export is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Mbox,
    Maildir,
    Eml,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Mbox, Format::Maildir, Format::Eml];

    /// The export target at `path`.
    pub fn target(self, path: PathBuf) -> Target {
        match self {
            Format::Mbox => Target::Mbox(path),
            Format::Maildir => Target::Maildir(path),
            Format::Eml => Target::Eml(path),
        }
    }

    /// Whether the target is one file, rather than a directory.
    pub fn is_file(self) -> bool {
        self == Format::Mbox
    }
}

/// A name for the export in `dir` that nothing uses yet, from the query and the format:
/// `mailo-inbox.mbox`, `mailo-inbox` for a Maildir, `mailo-inbox-eml` for a directory of files.
pub fn suggested(dir: &Path, query: &str, format: Format) -> PathBuf {
    let slug = slug(query);
    let name = match format {
        Format::Mbox => format!("mailo-{slug}.mbox"),
        Format::Maildir => format!("mailo-{slug}"),
        Format::Eml => format!("mailo-{slug}-eml"),
    };
    crate::attach::free_path(dir, &name)
}

/// The query as a file name: letters and digits, the rest one dash, at most forty characters.
fn slug(query: &str) -> String {
    let mut out = String::new();
    for ch in query.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.chars().count() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "mail".to_owned()
    } else {
        out.to_owned()
    }
}

/// How many messages `query` means, or why it means none.
pub fn selection_size(
    store: &SqliteStore,
    query: &str,
    now: DateTime<Utc>,
) -> Result<usize, CoreError> {
    Ok(export::select(store, query, now)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::new_account_id;
    use mail_domain::{AccountPlan, Folder, Holds, Subscription, presets};

    fn now() -> DateTime<Utc> {
        chrono::TimeZone::timestamp_opt(&Utc, 1_700_000_000, 0).unwrap()
    }

    #[test]
    fn a_typed_path_starts_at_home_with_a_tilde() {
        let home = Some(OsStr::new("/home/ann"));
        const CASES: &[(&str, &str)] = &[
            ("~", "/home/ann"),
            (
                "~/Downloads/All mail.mbox",
                "/home/ann/Downloads/All mail.mbox",
            ),
            ("  ~/Mail  ", "/home/ann/Mail"),
            ("/srv/mail", "/srv/mail"),
            ("~bob/Mail", "~bob/Mail"),
        ];
        for (typed, wanted) in CASES {
            assert_eq!(expand(typed, home), PathBuf::from(wanted), "{typed:?}");
        }
        assert_eq!(expand("~/Mail", None), PathBuf::from("~/Mail"));
        // Windows takes either slash as a separator, so a typed `~\` is home there too.
        #[cfg(windows)]
        assert_eq!(expand("~\\Mail", home), PathBuf::from("/home/ann/Mail"));
    }

    #[test]
    fn an_export_is_never_written_over_an_existing_file() {
        let out = tempfile::tempdir().unwrap();
        let first = suggested(out.path(), "from:dana has:attachment", Format::Mbox);
        assert_eq!(
            first,
            out.path().join("mailo-from-dana-has-attachment.mbox")
        );
        std::fs::write(&first, b"someone's archive").unwrap();
        let second = suggested(out.path(), "from:dana has:attachment", Format::Mbox);
        assert_eq!(
            second,
            out.path().join("mailo-from-dana-has-attachment (2).mbox")
        );
        std::fs::create_dir(out.path().join("mailo-inbox")).unwrap();
        assert_eq!(
            suggested(out.path(), "inbox", Format::Maildir),
            out.path().join("mailo-inbox (2)")
        );
        assert_eq!(
            suggested(out.path(), "  ", Format::Eml),
            out.path().join("mailo-mail-eml")
        );
    }

    #[test]
    fn what_cannot_be_read_is_refused_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(look(Path::new("")), Seen::Blank);
        assert_eq!(
            look(&dir.path().join("nowhere.mbox")),
            Seen::Refused(Refusal::NothingThere)
        );
        let empty = dir.path().join("empty.eml");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(look(&empty), Seen::Refused(Refusal::Empty));
        let plain_dir = dir.path().join("Photos");
        std::fs::create_dir(&plain_dir).unwrap();
        let Seen::Refused(Refusal::Unreadable(why)) = look(&plain_dir) else {
            panic!("a plain directory was taken for mail");
        };
        assert!(why.contains("not a Maildir"), "{why}");
    }

    #[test]
    fn mail_is_counted_where_it_lies() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Mail");
        for sub in ["cur", "new", "tmp"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        for (n, id) in ["one", "two"].into_iter().enumerate() {
            std::fs::write(
                root.join(format!("new/1699363{n}.M1P1Q1.host")),
                format!(
                    "Message-ID: <{id}@example.test>\nFrom: a@example.test\nTo: b@example.test\n\
                     Subject: {id}\nDate: Tue, 07 Nov 2023 13:20:51 +0000\n\nbody\n"
                ),
            )
            .unwrap();
        }
        assert_eq!(
            look(&root),
            Seen::Mail {
                source: Source::Maildir(root.clone()),
                count: 2
            }
        );
    }

    #[test]
    fn imported_mail_can_go_to_an_imap_accounts_folders_and_not_a_pop3_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path().join("blobs")).unwrap();
        let add = |id: AccountId, plan: &AccountPlan| {
            mail_store::testing::seed_account_plan(&store, id, &plan.address, plan, Some(now()));
        };
        let imap = new_account_id();
        let manual = presets::Manual {
            imap_host: "imap.nowhere.example".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.nowhere.example".to_owned(),
            smtp_port: 465,
            login: None,
        };
        add(
            imap.clone(),
            &presets::manual("me@nowhere.example", &manual, now()).plan,
        );
        let pop = presets::ManualPop3 {
            pop3_host: "pop.nowhere.example".to_owned(),
            pop3_port: 995,
            smtp_host: "smtp.nowhere.example".to_owned(),
            smtp_port: 465,
            login: None,
        };
        add(
            new_account_id(),
            &presets::manual_pop3("pop@nowhere.example", &pop, now()).plan,
        );
        let folder = |path: &str| Folder {
            account: imap.clone(),
            path: path.to_owned(),
            delimiter: Some('/'),
            special: None,
            subscription: Subscription::Subscribed,
            holds: Holds::Mail,
        };
        store
            .put_folders(imap.clone(), vec![folder("INBOX"), folder("Archive/2023")])
            .unwrap();
        let upload = |path: &str| Upload {
            account: imap.clone(),
            address: "me@nowhere.example".to_owned(),
            folder: path.to_owned(),
        };
        let wanted = vec![upload("Archive/2023"), upload("INBOX")];
        assert_eq!(upload_folders(&store), wanted);
        // Local folders are somewhere to import into, never an account to upload to.
        crate::account::local(&store, now()).unwrap();
        assert_eq!(upload_folders(&store), wanted);
    }
}
