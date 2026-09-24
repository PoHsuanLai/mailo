//! What the Import and Export sheets do, as functions of a store and a path.
//!
//! The sheets only draw what these answer, and the tests drive these rather than a window. The
//! work itself is `crate::import` and `crate::export`, the same functions `mailo import` and
//! `mailo export` run; this adds the words a person reads while choosing, and the one step the
//! command line takes after an upload — sending it — is [`import_then_send`].

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use mail_domain::{AccountId, Filter, Incoming};
use mail_store::{SqliteStore, Store};

use crate::export::{self, Exported, Target};
use crate::import::{self, Destination, Imported, Source};
use crate::view::{Shell, Source as Listed};

/// A typed path, with a leading `~` meaning `home`. Surrounding space is not part of a name
/// anybody types on purpose, so it is dropped.
pub(in crate::ui) fn expand(typed: &str, home: Option<&OsStr>) -> PathBuf {
    let typed = typed.trim();
    match (typed, home) {
        ("~", Some(home)) => PathBuf::from(home),
        (_, Some(home)) if typed.starts_with("~/") => PathBuf::from(home).join(&typed[2..]),
        _ => PathBuf::from(typed),
    }
}

/// [`expand`] against this user's home directory.
pub(in crate::ui) fn expand_here(typed: &str) -> PathBuf {
    expand(typed, std::env::var_os("HOME").as_deref())
}

/// What a typed path holds, as the sheet says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Looked {
    /// Nothing typed yet.
    Blank,
    /// Mail this can import: which kind, how many messages, and the line that says so.
    Mail {
        source: Source,
        count: usize,
        said: String,
    },
    /// Why it cannot be read, in words.
    Refused(String),
}

/// Look at `path` the way an import will read it, and count what is there.
///
/// Blocking and as slow as reading the source: an mbox is read to its end to count it.
pub(in crate::ui) fn look(path: &Path) -> Looked {
    if path.as_os_str().is_empty() {
        return Looked::Blank;
    }
    let shown = path.display();
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Looked::Refused(format!("There is nothing at {shown}."));
        }
        Err(e) => return Looked::Refused(format!("{shown} cannot be reached: {}.", plain(&e))),
    };
    if meta.is_file() {
        if let Err(e) = std::fs::File::open(path) {
            return Looked::Refused(format!("{shown} cannot be opened: {}.", plain(&e)));
        }
        if meta.len() == 0 {
            return Looked::Refused(format!("{shown} is empty: there is no mail in it."));
        }
    }
    let source = match import::detect(path) {
        Ok(source) => source,
        Err(why) => return Looked::Refused(sentence(&why)),
    };
    let count = match import::items(&source) {
        Ok(items) => items.count(),
        Err(why) => return Looked::Refused(sentence(&why)),
    };
    let kind = match source {
        Source::Mbox(_) => "mbox",
        Source::Maildir(_) => "Maildir",
        Source::Eml(_) => "One message file",
    };
    let said = format!("{kind}, {}", messages(count));
    Looked::Mail {
        source,
        count,
        said,
    }
}

/// An I/O error as a person reads it: no `os error 13`.
fn plain(e: &std::io::Error) -> String {
    match e.kind() {
        ErrorKind::PermissionDenied => "you do not have permission to read it".to_owned(),
        ErrorKind::NotFound => "it is not there".to_owned(),
        _ => {
            let text = e.to_string();
            match text.split_once(" (os error") {
                Some((words, _)) => words.to_lowercase(),
                None => text,
            }
        }
    }
}

/// A reason from `import` as a sentence: a capital letter and a full stop.
fn sentence(why: &str) -> String {
    let why = why.trim().trim_end_matches('.');
    let mut chars = why.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => "That cannot be read.".to_owned(),
    }
}

/// `1 message`, `1 204 messages`: thousands apart, the way a count is read aloud.
pub(in crate::ui) fn messages(count: usize) -> String {
    if count == 1 {
        "1 message".to_owned()
    } else {
        format!("{} messages", grouped(count))
    }
}

/// `1 204`: a number with its thousands apart, held together by no-break spaces.
pub(in crate::ui) fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(digit);
    }
    out
}

/// Where imported mail goes, as the sheet's menu offers it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::ui) enum Dest {
    /// The local-only account.
    #[default]
    Local,
    /// A folder on an IMAP account, uploaded through its outbox.
    Folder {
        account: AccountId,
        address: String,
        folder: String,
    },
}

impl Dest {
    /// The menu key that picks this.
    pub(in crate::ui) fn key(&self) -> String {
        match self {
            Dest::Local => "local".to_owned(),
            Dest::Folder {
                account, folder, ..
            } => format!("{account}\u{1f}{folder}"),
        }
    }

    /// What the destination button says.
    pub(in crate::ui) fn label(&self) -> String {
        match self {
            Dest::Local => "Local folders".to_owned(),
            Dest::Folder {
                address, folder, ..
            } => format!("{folder} on {address}"),
        }
    }

    fn destination(&self) -> Destination {
        match self {
            Dest::Local => Destination::Local,
            Dest::Folder {
                address, folder, ..
            } => Destination::Mailbox {
                account: address.clone(),
                folder: folder.clone(),
            },
        }
    }
}

/// Every place imported mail can go: local folders first, then each IMAP account's folders as
/// the store lists them, accounts oldest first. POP3 has no folders to upload into.
pub(in crate::ui) fn destinations(store: &SqliteStore) -> Vec<Dest> {
    let mut out = vec![Dest::Local];
    for row in super::super::data::account_rows(store) {
        if !matches!(row.plan.incoming, Incoming::Imap { .. }) {
            continue;
        }
        let mut folders: Vec<String> = store
            .folders(row.id)
            .unwrap_or_default()
            .into_iter()
            .map(|folder| folder.path)
            .collect();
        folders.sort();
        out.extend(folders.into_iter().map(|folder| Dest::Folder {
            account: row.id,
            address: row.address.clone(),
            folder,
        }));
    }
    out
}

/// What an import did, and whether an upload now waits in an account's outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Done {
    pub total: Imported,
    /// `import::said`, as one line.
    pub said: String,
    /// The account whose outbox holds the uploads. `None` when the mail was kept here.
    pub queued: Option<AccountId>,
}

/// Import `source` into `into`: kept in local folders, or queued for a folder's upload.
///
/// The window's function, and the one its tests drive. Nothing leaves here; see
/// [`import_then_send`].
pub(in crate::ui) fn import_now(
    store: &SqliteStore,
    source: &Source,
    into: &Dest,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<Done, String> {
    let (total, queued) = match into {
        Dest::Local => (import::into_local(store, source, now, progress)?, None),
        Dest::Folder {
            address, folder, ..
        } => {
            let (account, total) =
                import::queue_uploads(store, address, folder, source, now, progress)?;
            (total, Some(account))
        }
    };
    let said = import::said(&total, &into.destination()).trim().to_owned();
    Ok(Done {
        total,
        said,
        queued,
    })
}

/// [`import_now`], then an upload sent while the person watches, as `mailo import
/// --to-mailbox` does. Whatever does not go stays queued for the next sync, and the sentence
/// says so.
pub(in crate::ui) fn import_then_send(
    store: &Arc<SqliteStore>,
    source: &Source,
    into: &Dest,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<String, String> {
    let done = import_now(store, source, into, now, progress)?;
    let Some(account) = done.queued else {
        return Ok(done.said);
    };
    let sent = match crate::sync::drain(store, account, now) {
        Ok(report) => {
            let mut out = format!("{}. {} uploaded", done.said, report.appended);
            if report.still_queued > 0 {
                out.push_str(&format!(
                    "; {} still queued, the next sync tries again",
                    report.still_queued
                ));
            }
            for note in &report.needs_attention {
                out.push_str(&format!(". Needs attention: {note}"));
            }
            out
        }
        Err(why) => format!(
            "{}. Not uploaded yet: {why}. It stays queued for the next sync.",
            done.said
        ),
    };
    Ok(sent)
}

/// The three ways an export is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui) enum Format {
    #[default]
    Mbox,
    Maildir,
    Eml,
}

impl Format {
    pub(in crate::ui) const ALL: [Format; 3] = [Format::Mbox, Format::Maildir, Format::Eml];

    /// The segment's name.
    pub(in crate::ui) fn label(self) -> &'static str {
        match self {
            Format::Mbox => "Mbox",
            Format::Maildir => "Maildir",
            Format::Eml => ".eml",
        }
    }

    /// The export target at `path`.
    pub(in crate::ui) fn target(self, path: PathBuf) -> Target {
        match self {
            Format::Mbox => Target::Mbox(path),
            Format::Maildir => Target::Maildir(path),
            Format::Eml => Target::Eml(path),
        }
    }

    /// Whether the target is one file, rather than a directory.
    pub(in crate::ui) fn is_file(self) -> bool {
        self == Format::Mbox
    }
}

/// What the export field opens with: the search being shown, or else the place's name in the
/// words `export::select` reads.
pub(in crate::ui) fn prefill(shell: &Shell) -> String {
    let search = shell.search.trim();
    if !search.is_empty() {
        return search.to_owned();
    }
    let Some(place) = shell.places.get(shell.selected) else {
        return "inbox".to_owned();
    };
    let label = match &place.source {
        Listed::Mail(Filter::HasLabel(_)) => Some(place.name.clone()),
        _ => None,
    };
    if let Some(name) = label {
        return if name.contains(char::is_whitespace) {
            format!("label:\"{name}\"")
        } else {
            format!("label:{name}")
        };
    }
    match place.name.as_str() {
        "Starred" => "is:starred".to_owned(),
        "Snoozed" => "is:snoozed".to_owned(),
        "Pinned" => "is:pinned".to_owned(),
        other => other.to_lowercase(),
    }
}

/// A name for the export in `dir` that nothing uses yet, from the query and the format:
/// `mailo-inbox.mbox`, `mailo-inbox` for a Maildir, `mailo-inbox-eml` for a directory of files.
pub(in crate::ui) fn suggested(dir: &Path, query: &str, format: Format) -> PathBuf {
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

/// How many messages `query` means, as the sheet says it, or why it means none.
pub(in crate::ui) fn counted(store: &SqliteStore, query: &str, now: DateTime<Utc>) -> Counted {
    if query.trim().is_empty() {
        return Counted::Blank;
    }
    match export::select(store, query, now) {
        Ok(ids) => Counted::Some(ids.len()),
        Err(why) => Counted::Refused(sentence(&why)),
    }
}

/// The export field's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Counted {
    Blank,
    Some(usize),
    Refused(String),
}

/// Export what `query` means to `target`: the window's function, and the one its tests drive.
pub(in crate::ui) fn export_now(
    store: &SqliteStore,
    query: &str,
    target: &Target,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Exported),
) -> Result<(Exported, String), String> {
    let chosen = export::select(store, query, now).map_err(|why| sentence(&why))?;
    if chosen.is_empty() {
        return Err("Nothing matches, so nothing was written.".to_owned());
    }
    let done = export::export(store, &chosen, target, now, progress)?;
    let said = export::said(&done, target)
        .lines()
        .collect::<Vec<_>>()
        .join(" ");
    Ok((done, said))
}
