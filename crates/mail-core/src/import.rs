//! `mailo import`: mail from files — an mbox, a Maildir, a single `.eml` — into this client.
//!
//! By default into the local-only account ("local folders"), created on first use: mail kept on
//! this computer and never synced, the way other clients keep Local Folders. The alternative,
//! `--to-mailbox`, uploads each message into a mailbox on an IMAP account through the outbox,
//! so the server holds it and every other device sees it.
//!
//! Local mail goes through the same parse, identity and threading as a sync
//! ([`mail_runtime::assemble::keep`]), so it is searchable and threads with what is already
//! here. Both ways are idempotent: a message is recognised by its identity — its `Message-ID`,
//! or a digest where it has none — and importing the same file twice keeps or uploads nothing
//! the second time.
//!
//! Sources are read one message at a time; a Takeout archive of several gigabytes never has to
//! fit in memory, and a batch is written every [`BATCH`] messages with a progress line.

use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::{AccountId, ChangeId, Incoming, MailboxRef, Patch, ProtoOp, RemoteIntent};
use mail_mime::archive::{self, Placement, Sniffed, maildir, mbox};
use mail_runtime::assemble::{self, Keep};
use mail_store::{SqliteStore, Store};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// How many messages are written to the store at once.
pub const BATCH: usize = 500;

/// What a path holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Mbox(PathBuf),
    Maildir(PathBuf),
    Eml(PathBuf),
}

/// Where imported mail goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// The local-only account.
    Local,
    /// A mailbox on an IMAP account, by the account's address and the folder's path.
    Mailbox { account: String, folder: String },
}

/// Which format `path` is: a directory with `cur`, `new` or `tmp` in it is a Maildir, a file that
/// opens with a `From ` line is an mbox, and any other file is one message.
pub fn detect(path: &Path) -> Result<Source, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        let maildir = ["cur", "new", "tmp"]
            .iter()
            .any(|sub| path.join(sub).is_dir());
        return if maildir {
            Ok(Source::Maildir(path.to_owned()))
        } else {
            Err(format!(
                "{} is a directory but not a Maildir: it has no cur, new or tmp inside",
                path.display()
            ))
        };
    }
    let mut head = [0u8; 512];
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let n = read_up_to(&mut file, &mut head).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(match archive::sniff(&head[..n]) {
        Sniffed::Mbox => Source::Mbox(path.to_owned()),
        Sniffed::Eml => Source::Eml(path.to_owned()),
    })
}

fn read_up_to(file: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// One message out of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// In CRLF line endings: RFC 5322's, and what an IMAP server wants in an upload.
    pub raw: Vec<u8>,
    pub placement: Placement,
    /// When the source says it arrived: the mbox envelope's date, or the time in a Maildir
    /// file's name. The upload's internal date, with the `Date` header as the fallback.
    pub received: Option<DateTime<Utc>>,
}

/// The messages in `source`, one at a time.
pub fn items(source: &Source) -> Result<Box<dyn Iterator<Item = Result<Item, String>>>, String> {
    match source {
        Source::Eml(path) => {
            let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Box::new(std::iter::once(Ok(Item {
                raw: archive::crlf(&raw),
                // One message on its own says nothing about where it was: the inbox, read.
                placement: archive::folder_placement(None, vec![mail_domain::SystemFlag::Seen]),
                received: None,
            }))))
        }
        Source::Mbox(path) => {
            let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let folder = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned());
            let shown = path.display().to_string();
            let reader = mbox::Reader::new(BufReader::with_capacity(1 << 16, file));
            Ok(Box::new(reader.map(move |message| {
                let message = message.map_err(|e| format!("{shown}: {e}"))?;
                Ok(Item {
                    placement: mbox::placement(&message.raw, folder.as_deref()),
                    raw: archive::crlf(&message.raw),
                    received: message.envelope.date,
                })
            })))
        }
        Source::Maildir(root) => {
            let files = maildir_files(root)?;
            Ok(Box::new(files.into_iter().map(|found| {
                let raw = std::fs::read(&found.path)
                    .map_err(|e| format!("{}: {e}", found.path.display()))?;
                Ok(Item {
                    raw: archive::crlf(&raw),
                    placement: maildir::placement(
                        found.folder.as_deref(),
                        found.sub,
                        &found.name.flags,
                    ),
                    received: delivered_at(&found.name.unique),
                })
            })))
        }
    }
}

/// A message file in a Maildir, and where in it.
struct Found {
    path: PathBuf,
    folder: Option<String>,
    sub: maildir::Sub,
    name: maildir::Name,
}

/// Every message file under a Maildir and its Maildir++ folders, in name order.
///
/// The names only, not the contents: a list of paths is small next to the mail it names.
fn maildir_files(root: &Path) -> Result<Vec<Found>, String> {
    let mut places: Vec<(PathBuf, Option<String>)> = vec![(root.to_owned(), None)];
    let entries = std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut folders: Vec<(PathBuf, String)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            maildir::folder_of_dir(&name).map(|folder| (entry.path(), folder))
        })
        .collect();
    folders.sort();
    places.extend(
        folders
            .into_iter()
            .map(|(path, folder)| (path, Some(folder))),
    );

    let mut out = Vec::new();
    for (dir, folder) in places {
        for (sub, name) in [(maildir::Sub::New, "new"), (maildir::Sub::Cur, "cur")] {
            let Ok(entries) = std::fs::read_dir(dir.join(name)) else {
                continue;
            };
            let mut files: Vec<(String, PathBuf)> = entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_file())
                .map(|entry| {
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        entry.path(),
                    )
                })
                // Dot files are not deliveries: editors' swap files, `.DS_Store`.
                .filter(|(file, _)| !file.starts_with('.'))
                .collect();
            files.sort();
            out.extend(files.into_iter().map(|(file, path)| Found {
                path,
                folder: folder.clone(),
                sub,
                name: maildir::parse_name(&file),
            }));
        }
    }
    Ok(out)
}

/// The delivery time a Maildir unique name begins with, in seconds since the epoch.
fn delivered_at(unique: &str) -> Option<DateTime<Utc>> {
    let secs: i64 = unique.split('.').next()?.parse().ok()?;
    DateTime::from_timestamp(secs, 0)
}

/// What an import did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    /// Messages read out of the source.
    pub read: usize,
    /// New here: kept locally, or queued for upload.
    pub added: usize,
    /// Already held (or already queued), by identity.
    pub already: usize,
    /// Bytes that were not a message.
    pub unreadable: usize,
}

/// Import `source` into the local-only account, creating the account on first use.
///
/// `progress` is called after every batch with the running totals.
pub fn into_local(
    store: &SqliteStore,
    source: &Source,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<Imported, String> {
    let account = crate::account::local(store, now)?;
    let mut total = Imported::default();
    let mut batch: Vec<Keep> = Vec::with_capacity(BATCH);
    for item in items(source)? {
        let item = item?;
        total.read += 1;
        batch.push(Keep {
            raw: item.raw,
            placement: item.placement,
        });
        if batch.len() == BATCH {
            keep(store, account, std::mem::take(&mut batch), now, &mut total)?;
            progress(&total);
        }
    }
    if !batch.is_empty() {
        keep(store, account, batch, now, &mut total)?;
        progress(&total);
    }
    Ok(total)
}

fn keep(
    store: &SqliteStore,
    account: AccountId,
    batch: Vec<Keep>,
    now: DateTime<Utc>,
    total: &mut Imported,
) -> Result<(), String> {
    let kept = assemble::keep(store, account, batch, now).map_err(|e| e.to_string())?;
    total.added += kept.added;
    total.already += kept.already;
    total.unreadable += kept.unreadable;
    Ok(())
}

/// Queue every message in `source` for upload into `folder` on the IMAP account `address`.
///
/// Nothing is sent here; [`crate::sync::drain`] sends it. A message the account already holds,
/// by identity, is not queued, and neither is one already waiting in the outbox for the same
/// folder: that is what makes running the same import twice upload each message once.
///
/// Returns the account, so the caller can drain its outbox, and what was queued.
pub fn queue_uploads(
    store: &SqliteStore,
    address: &str,
    folder: &str,
    source: &Source,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<(AccountId, Imported), String> {
    let address = address.to_lowercase();
    let account = crate::sync::configured(store)?
        .into_iter()
        .find(|a| a.address == address)
        .ok_or_else(|| {
            format!("no account for {address:?}. `mailo account list` says which there are.")
        })?;
    if !matches!(account.plan.incoming, Incoming::Imap { .. }) {
        return Err(format!(
            "{address} is not an IMAP account, so it has no mailboxes to upload into; \
             leave out --to-mailbox to keep the mail on this computer"
        ));
    }
    let listed = store.folders(account.id).map_err(|e| e.to_string())?;
    if !listed.is_empty() && !listed.iter().any(|f| f.path == folder) {
        return Err(format!(
            "{address} has no folder called {folder:?}; `mailo folder list {address}` shows \
             them, and `mailo folder new {address} {folder}` makes one"
        ));
    }
    let mailbox = MailboxRef {
        account: account.id,
        path: folder.to_owned(),
    };
    // Uploads already waiting, from an import whose drain did not finish.
    let mut waiting: Vec<mail_domain::BlobId> = store
        .outbox_due(
            account.id,
            now + TimeDelta::try_days(3650).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|entry| match entry.op {
            ProtoOp::Append {
                mailbox: queued,
                raw,
                ..
            } if queued == mailbox => Some(raw),
            _ => None,
        })
        .collect();

    let mut total = Imported::default();
    for item in items(source)? {
        let item = item?;
        total.read += 1;
        let Ok(fields) = mail_mime::parse(&item.raw) else {
            total.unreadable += 1;
            continue;
        };
        let key = assemble::identity(&fields, &item.raw);
        if store.holds(account.id, &key).map_err(|e| e.to_string())? {
            total.already += 1;
            continue;
        }
        let raw = store
            .blobs()
            .put(&store.connection(), &item.raw)
            .map_err(|e| e.to_string())?;
        if waiting.contains(&raw) {
            total.already += 1;
            continue;
        }
        store
            .enqueue(
                account.id,
                RemoteIntent::Append {
                    mailbox: mailbox.clone(),
                    flags: item.placement.flags.clone(),
                    date: item.received.or(fields.date),
                    raw,
                },
                // Nothing to undo: an upload changes nothing here until it has happened.
                &Patch {
                    id: ChangeId::generate(),
                    changes: Vec::new(),
                },
                now,
            )
            .map_err(|e| e.to_string())?;
        waiting.push(raw);
        total.added += 1;
        if total.read % BATCH == 0 {
            progress(&total);
        }
    }
    progress(&total);
    Ok((account.id, total))
}

/// What an import says when it is done.
pub fn said(total: &Imported, into: &Destination) -> String {
    let place = match into {
        Destination::Local => "kept in local folders".to_owned(),
        Destination::Mailbox { account, folder } => format!("queued for {folder} on {account}"),
    };
    let mut out = format!(
        "{} message(s) read; {} {place}; {} already there",
        total.read, total.added, total.already
    );
    if total.unreadable > 0 {
        out.push_str(&format!("; {} not a message, skipped", total.unreadable));
    }
    out.push('\n');
    out
}
