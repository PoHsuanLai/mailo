//! `mailo import`: mail from files — an mbox, a Maildir, a single `.eml`, Apple Mail's mailboxes
//! and `.emlx` files, a Thunderbird profile — into this client.
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

use crate::error::CoreError;
use crate::export;
use crate::formats::{self, Archive, Writer};
use chrono::{DateTime, TimeDelta, Utc};
use mail_archive::{self as archive, Placement, Sniffed, emlx, maildir, mbox, thunderbird};
use mail_domain::{ChangeId, MailboxRef, Patch, ProtoOp, RemoteIntent};
use mail_runtime::assemble::{self, Keep};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// How many messages are written to the store at once.
pub const BATCH: usize = 500;

/// What a path holds: the format it was found to be, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub archive: &'static dyn Archive,
    pub path: PathBuf,
}

impl Source {
    pub fn new(archive: &'static dyn Archive, path: PathBuf) -> Self {
        Self { archive, path }
    }

    /// The messages in this source, one at a time.
    pub fn items(&self) -> Result<Items, CoreError> {
        self.archive.items(&self.path)
    }
}

/// One source's messages, read one at a time.
pub type Items = Box<dyn Iterator<Item = Result<Item, CoreError>>>;

/// Where imported mail goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// The local-only account.
    Local,
    /// A mailbox on an IMAP account, by the account's address and the folder's path.
    Mailbox { account: String, folder: String },
}

/// Which format `path` is: the first of [`formats::ARCHIVES`] to claim it.
pub fn detect(path: &Path) -> Result<Source, CoreError> {
    Ok(Source::new(formats::detect(path)?, path.to_owned()))
}

/// How deep under a chosen directory Apple Mail's packages are looked for: its library keeps
/// them a version and an account down (`V10/<account>/INBOX.mbox`), and an export at the top.
const APPLE_DEPTH: usize = 3;

/// What the first bytes of the file at `path` say it is.
fn sniff(path: &Path) -> Result<Sniffed, CoreError> {
    let mut head = [0u8; 512];
    let mut file = std::fs::File::open(path).map_err(CoreError::at(&path))?;
    let n = read_up_to(&mut file, &mut head).map_err(CoreError::at(&path))?;
    Ok(archive::sniff(&head[..n]))
}

/// The name of the directory `path` stands for. `.` and `..` have none of their own, so those
/// are looked up for the directory they resolve to.
fn dir_name(path: &Path) -> Option<String> {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    // A path that cannot be resolved names nothing, and the folder is then unnamed.
    name(path).or_else(|| path.canonicalize().ok().and_then(|real| name(&real)))
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

/// A directory's entries, sorted by name, with whether each is a directory. Links are not
/// followed: a library that links back into itself must not be walked for ever.
fn entries(dir: &Path) -> Result<Vec<(String, PathBuf, bool)>, CoreError> {
    let read = std::fs::read_dir(dir).map_err(CoreError::at(&dir))?;
    let mut out: Vec<(String, PathBuf, bool)> = read
        // An entry that cannot be read is one this could not open either.
        .filter_map(Result::ok)
        .filter_map(|entry| {
            // An entry whose kind cannot be read is one this could not open either.
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() {
                return None;
            }
            Some((
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
                kind.is_dir(),
            ))
        })
        .collect();
    out.sort();
    Ok(out)
}

/// `dir`'s entries, and if it cannot be listed, the error naming `folder` as well as the path.
fn folder_entries(
    dir: &Path,
    folder: Option<&str>,
) -> Result<Vec<(String, PathBuf, bool)>, CoreError> {
    entries(dir).map_err(|e| match folder {
        Some(folder) => CoreError::cannot(format!("list the folder {folder}"), e),
        None => e,
    })
}

/// Whether `listed` holds a `.msf` index, which marks a Thunderbird account directory.
fn has_index(listed: &[(String, PathBuf, bool)]) -> bool {
    listed
        .iter()
        .any(|(name, _, is_dir)| !is_dir && name.ends_with(".msf"))
}

/// Whether `dir` is a Thunderbird profile, or a directory of its account folders, or one.
fn is_thunderbird(dir: &Path) -> bool {
    let index = |dir: &Path| {
        // A directory that cannot be listed holds no index this could read.
        has_index(&entries(dir).unwrap_or_default())
    };
    let profile = dir.join("prefs.js").is_file()
        && (dir.join("Mail").is_dir() || dir.join("ImapMail").is_dir());
    profile
        || index(dir)
        // `Mail` or `ImapMail` itself: account directories, each with its indexes.
        || entries(dir)
            // As above: nothing listed, nothing found.
            .unwrap_or_default()
            .iter()
            .any(|(_, path, is_dir)| *is_dir && index(path))
}

/// Whether `dir` is an Apple Mail package, or holds one or an `.emlx` within `depth` levels.
fn is_apple_mail(dir: &Path, depth: usize) -> bool {
    if dir_name(dir).is_some_and(|name| emlx::folder_of_package(&name).is_some()) {
        return true;
    }
    // A directory that cannot be listed holds nothing of Apple Mail's this could read.
    let found = entries(dir).unwrap_or_default();
    found.iter().any(|(name, path, is_dir)| {
        if *is_dir {
            emlx::folder_of_package(name).is_some() || (depth > 0 && is_apple_mail(path, depth - 1))
        } else {
            has_extension(path, "emlx")
        }
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
    pub completeness: Completeness,
}

/// How much of a message its source held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    Whole,
    /// An Apple Mail `.partial.emlx`: the message without the attachments stored beside it.
    /// Imported as it is, and counted ([`Imported::partial`]).
    WithoutAttachments,
    /// A Thunderbird message of which only a part of the body was kept. Not imported, since a
    /// cut-off body kept under the message's identity would stand in for the whole message when
    /// it is imported complete; counted ([`Imported::cut_off`]).
    CutOff,
}

impl Item {
    /// Count this item as read, and say whether it is to be kept: a cut-off message is not.
    fn tally(&self, total: &mut Imported) -> bool {
        total.read += 1;
        match self.completeness {
            Completeness::Whole => true,
            Completeness::WithoutAttachments => {
                total.partial += 1;
                true
            }
            Completeness::CutOff => {
                total.cut_off += 1;
                false
            }
        }
    }
}

/// An mbox file: the format that opens with a `From ` line.
#[derive(Debug)]
pub struct Mbox;

impl Archive for Mbox {
    fn slug(&self) -> &'static str {
        "mbox"
    }

    fn claims(&self, path: &Path) -> Result<bool, CoreError> {
        Ok(!path.is_dir() && !has_extension(path, "emlx") && sniff(path)? == Sniffed::Mbox)
    }

    fn items(&self, path: &Path) -> Result<Items, CoreError> {
        mbox_items(path, mbox_folder(path), Mailer::Any)
    }

    fn writer(&self) -> Option<&'static dyn Writer> {
        Some(&export::MboxWriter)
    }
}

/// A Maildir: a directory with `cur`, `new` or `tmp` in it.
#[derive(Debug)]
pub struct Maildir;

impl Archive for Maildir {
    fn slug(&self) -> &'static str {
        "maildir"
    }

    fn claims(&self, path: &Path) -> Result<bool, CoreError> {
        Ok(["cur", "new", "tmp"]
            .iter()
            .any(|sub| path.join(sub).is_dir()))
    }

    fn items(&self, root: &Path) -> Result<Items, CoreError> {
        let files = maildir_files(root)?;
        Ok(Box::new(files.into_iter().map(|found| {
            let raw = std::fs::read(&found.path).map_err(CoreError::at(&found.path))?;
            Ok(Item {
                raw: archive::crlf(&raw),
                placement: maildir::placement(
                    found.folder.as_deref(),
                    found.sub,
                    &found.name.flags,
                ),
                received: delivered_at(&found.name.unique),
                completeness: Completeness::Whole,
            })
        })))
    }

    fn writer(&self) -> Option<&'static dyn Writer> {
        Some(&export::MaildirWriter)
    }
}

/// A single message file.
#[derive(Debug)]
pub struct Eml;

impl Archive for Eml {
    fn slug(&self) -> &'static str {
        "eml"
    }

    fn claims(&self, path: &Path) -> Result<bool, CoreError> {
        Ok(!path.is_dir() && !has_extension(path, "emlx") && sniff(path)? == Sniffed::Eml)
    }

    fn items(&self, path: &Path) -> Result<Items, CoreError> {
        let raw = std::fs::read(path).map_err(CoreError::at(&path))?;
        Ok(Box::new(std::iter::once(Ok(Item {
            raw: archive::crlf(&raw),
            // One message on its own says nothing about where it was: the inbox, read.
            placement: archive::folder_placement(None, vec![mail_domain::SystemFlag::Seen]),
            received: None,
            completeness: Completeness::Whole,
        }))))
    }

    fn writer(&self) -> Option<&'static dyn Writer> {
        Some(&export::EmlWriter)
    }
}

/// A Thunderbird profile, its `Mail` or `ImapMail` directory, or one account's in them: a
/// directory with Thunderbird's indexes or a profile's `prefs.js`.
#[derive(Debug)]
pub struct Thunderbird;

impl Archive for Thunderbird {
    fn slug(&self) -> &'static str {
        "thunderbird"
    }

    fn claims(&self, path: &Path) -> Result<bool, CoreError> {
        Ok(path.is_dir() && is_thunderbird(path))
    }

    fn items(&self, root: &Path) -> Result<Items, CoreError> {
        let mut found = Vec::new();
        for account in thunderbird_accounts(root)? {
            let imap = dir_name(account.parent().unwrap_or(Path::new(".")))
                .is_some_and(|home| home == "ImapMail");
            thunderbird_files(&account, None, imap, &mut found)?;
        }
        Ok(chained(found, Mailer::Thunderbird))
    }
}

/// Apple Mail's mailboxes: what its Export Mailbox writes (a `Name.mbox` package with an mbox
/// inside, or a folder of them), or its own library of `.emlx` files.
#[derive(Debug)]
pub struct AppleMail;

impl Archive for AppleMail {
    fn slug(&self) -> &'static str {
        "apple-mail"
    }

    fn claims(&self, path: &Path) -> Result<bool, CoreError> {
        Ok(path.is_dir() && is_apple_mail(path, APPLE_DEPTH))
    }

    fn items(&self, root: &Path) -> Result<Items, CoreError> {
        let mut found = Vec::new();
        let top = dir_name(root).and_then(|name| emlx::folder_of_package(&name).map(str::to_owned));
        apple_files(root, top, APPLE_DEPTH + 1, &mut found)?;
        Ok(chained(found, Mailer::Any))
    }
}

/// One of Apple Mail's own message files.
#[derive(Debug)]
pub struct Emlx;

impl Archive for Emlx {
    fn slug(&self) -> &'static str {
        "emlx"
    }

    fn claims(&self, path: &Path) -> Result<bool, CoreError> {
        Ok(!path.is_dir() && has_extension(path, "emlx"))
    }

    fn items(&self, path: &Path) -> Result<Items, CoreError> {
        // Where one file on its own was kept is not known: Apple Mail's flags place it.
        Ok(Box::new(std::iter::once(emlx_item(path, None))))
    }
}

/// The folder an mbox file holds, by its name: `Sent.mbox` is `Sent`. The `mbox` inside an Apple
/// Mail package is named for its package.
fn mbox_folder(path: &Path) -> Option<String> {
    // A bare `mbox` has the empty path as its parent, which is the current directory.
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let package = dir_name(parent);
    if path.file_name().is_some_and(|name| name == "mbox")
        && let Some(folder) = package.as_deref().and_then(emlx::folder_of_package)
    {
        return Some(folder.to_owned());
    }
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

/// Which program wrote an mbox, where that changes how it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mailer {
    Any,
    /// Thunderbird leaves deleted messages in the file, marked, until it is compacted.
    Thunderbird,
}

/// The messages in the mbox at `path`, placed in `folder`.
fn mbox_items(path: &Path, folder: Option<String>, mailer: Mailer) -> Result<Items, CoreError> {
    let file = std::fs::File::open(path).map_err(CoreError::at(&path))?;
    let shown = path.display().to_string();
    let reader = mbox::Reader::new(BufReader::with_capacity(1 << 16, file));
    Ok(Box::new(reader.filter_map(move |message| {
        let message = match message {
            Ok(message) => message,
            Err(e) => return Some(Err(CoreError::context(shown.clone(), e))),
        };
        let thunderbird = mailer == Mailer::Thunderbird;
        if thunderbird && thunderbird::expunged(&message.raw) {
            return None;
        }
        let cut = thunderbird && thunderbird::truncated(&message.raw);
        Some(Ok(Item {
            completeness: if cut {
                Completeness::CutOff
            } else {
                Completeness::Whole
            },
            placement: mbox::placement(&message.raw, folder.as_deref()),
            raw: archive::crlf(&message.raw),
            received: message.envelope.date,
        }))
    })))
}

/// One Apple Mail message file, placed in `folder`.
fn emlx_item(path: &Path, folder: Option<&str>) -> Result<Item, CoreError> {
    let bytes = std::fs::read(path).map_err(CoreError::at(&path))?;
    let split = emlx::split(&bytes).ok_or_else(|| CoreError::NotEmlx(path.to_owned()))?;
    Ok(Item {
        raw: archive::crlf(split.raw),
        placement: emlx::placement(folder, split.flags),
        received: split.received,
        completeness: if path
            .file_name()
            .is_some_and(|name| emlx::is_partial(&name.to_string_lossy()))
        {
            Completeness::WithoutAttachments
        } else {
            Completeness::Whole
        },
    })
}

/// A file of mail found under a directory, and the folder it holds.
#[derive(Debug)]
enum Held {
    Mbox(PathBuf, Option<String>),
    Emlx(PathBuf, Option<String>),
}

/// The messages in every file of `found`, file after file, each opened only when reached.
fn chained(found: Vec<Held>, mailer: Mailer) -> Items {
    Box::new(
        found
            .into_iter()
            .flat_map(move |held| -> Box<dyn Iterator<Item = _>> {
                match held {
                    Held::Mbox(path, folder) => match mbox_items(&path, folder, mailer) {
                        Ok(items) => items,
                        Err(e) => Box::new(std::iter::once(Err(e))),
                    },
                    Held::Emlx(path, folder) => {
                        Box::new(std::iter::once(emlx_item(&path, folder.as_deref())))
                    }
                }
            }),
    )
}

/// `parent/name`, or `name` at the top.
fn nested(parent: Option<&str>, name: &str) -> String {
    match parent {
        Some(parent) => format!("{parent}/{name}"),
        None => name.to_owned(),
    }
}

/// Every mbox and `.emlx` under `dir`, in name order, each with the mailbox it is in: the
/// `Name.mbox` packages it sits inside, outermost first, so `Work.mbox/2019.mbox` is `Work/2019`.
///
/// Inside a package Apple Mail keeps its library's files a few directories down; those
/// directories are its storage, not mailboxes, and only `.mbox` ones change the folder.
fn apple_files(
    dir: &Path,
    folder: Option<String>,
    depth: usize,
    found: &mut Vec<Held>,
) -> Result<(), CoreError> {
    for (name, path, is_dir) in folder_entries(dir, folder.as_deref())? {
        if is_dir {
            match emlx::folder_of_package(&name) {
                Some(package) => {
                    let inner = nested(folder.as_deref(), package);
                    apple_files(&path, Some(inner), APPLE_DEPTH + 1, found)?;
                }
                // Storage inside a package goes as deep as Apple Mail makes it; above the
                // packages, only as deep as they are looked for.
                None if folder.is_some() || depth > 0 => {
                    apple_files(&path, folder.clone(), depth.saturating_sub(1), found)?;
                }
                None => {}
            }
        } else if name == "mbox" && matches!(sniff(&path)?, Sniffed::Mbox) {
            found.push(Held::Mbox(path, folder.clone()));
        } else if has_extension(&path, "emlx") {
            found.push(Held::Emlx(path, folder.clone()));
        }
    }
    Ok(())
}

/// The account directories under a Thunderbird `root`: each under a profile's `Mail` and
/// `ImapMail`, `root` itself when it holds indexes, or else each directory in it.
fn thunderbird_accounts(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let homes: Vec<PathBuf> = ["Mail", "ImapMail"]
        .iter()
        .map(|home| root.join(home))
        .filter(|home| home.is_dir())
        .collect();
    let listed = entries(root)?;
    if homes.is_empty() && has_index(&listed) {
        return Ok(vec![root.to_owned()]);
    }
    let homes = if homes.is_empty() {
        vec![root.to_owned()]
    } else {
        homes
    };
    let mut out = Vec::new();
    for home in homes {
        out.extend(
            entries(&home)?
                .into_iter()
                .filter(|(_, _, is_dir)| *is_dir)
                .map(|(_, path, _)| path),
        );
    }
    Ok(out)
}

/// Every folder's mbox in a Thunderbird account directory, subfolders after their parent:
/// `Work.sbd/2019` is `Work/2019`. An empty file is a folder with nothing kept offline.
///
/// An IMAP account's folders (`imap`, under `ImapMail`) are named in modified UTF-7, as on the
/// wire, in their file and directory names: `Entw&APw-rfe` is `Entwürfe`. A local folder's name
/// is its own.
fn thunderbird_files(
    dir: &Path,
    parent: Option<&str>,
    imap: bool,
    found: &mut Vec<Held>,
) -> Result<(), CoreError> {
    let shown = |name: &str| {
        if imap {
            mail_proto::mutf7::decode(name)
        } else {
            name.to_owned()
        }
    };
    let listed = folder_entries(dir, parent)?;
    for (name, path, is_dir) in &listed {
        if *is_dir || !thunderbird::may_be_folder(name) {
            continue;
        }
        let empty = std::fs::metadata(path).map_err(CoreError::at(path))?.len() == 0;
        if !empty && matches!(sniff(path)?, Sniffed::Mbox) {
            found.push(Held::Mbox(path.clone(), Some(nested(parent, &shown(name)))));
        }
    }
    for (name, path, is_dir) in &listed {
        if let Some(folder) = thunderbird::folder_of_children(name).filter(|_| *is_dir) {
            thunderbird_files(path, Some(&nested(parent, &shown(folder))), imap, found)?;
        }
    }
    Ok(())
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
fn maildir_files(root: &Path) -> Result<Vec<Found>, CoreError> {
    let mut places: Vec<(PathBuf, Option<String>)> = vec![(root.to_owned(), None)];
    let entries = std::fs::read_dir(root).map_err(CoreError::at(&root))?;
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
    /// Read from Apple Mail's `.partial.emlx` files: imported without the attachments stored
    /// beside them.
    pub partial: usize,
    /// Thunderbird messages of which only a part of the body was kept, and so left out.
    pub cut_off: usize,
}

/// Import `source` into the local-only account, creating the account on first use.
///
/// `progress` is called after every batch with the running totals.
pub fn into_local(
    store: &SqliteStore,
    source: &Source,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<Imported, CoreError> {
    let account = crate::account::local(store, now)?;
    let mut total = Imported::default();
    let mut batch: Vec<Keep> = Vec::with_capacity(BATCH);
    for item in source.items()? {
        let item = item?;
        if !item.tally(&mut total) {
            continue;
        }
        batch.push(Keep {
            raw: item.raw,
            placement: item.placement,
        });
        if batch.len() == BATCH {
            keep(
                store,
                account.clone(),
                std::mem::take(&mut batch),
                now,
                &mut total,
            )?;
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
) -> Result<(), CoreError> {
    let kept = assemble::keep(store, account, batch, now)?;
    total.added += kept.added;
    total.already += kept.already;
    total.unreadable += kept.unreadable;
    Ok(())
}

/// Queue every message in `source` for upload into `folder` on the IMAP account `address`.
///
/// Nothing is sent here; [`crate::SyncOps::drain`] sends it. A message the account already holds,
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
) -> Result<(AccountId, Imported), CoreError> {
    let address = address.to_lowercase();
    let account = crate::sync::configured(store)?
        .into_iter()
        .find(|a| a.address == address)
        .ok_or_else(|| CoreError::NoAccountFor {
            address: address.clone(),
        })?;
    if !account.plan.incoming.caps().writes_mailboxes {
        return Err(CoreError::NotImap { address });
    }
    let listed = store.folders(account.id.clone())?;
    if !listed.is_empty() && !listed.iter().any(|f| f.path == folder) {
        return Err(CoreError::NoFolder {
            address,
            folder: folder.to_owned(),
        });
    }
    let mailbox = MailboxRef {
        account: account.id.clone(),
        path: folder.to_owned(),
    };
    // Uploads already waiting, from an import whose drain did not finish.
    let mut waiting: Vec<mail_domain::BlobId> = store
        .outbox_due(
            account.id.clone(),
            now + TimeDelta::try_days(3650).unwrap_or_default(),
        )?
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
    for item in source.items()? {
        let item = item?;
        if !item.tally(&mut total) {
            continue;
        }
        let Ok(fields) = mail_mime::parse(&item.raw) else {
            total.unreadable += 1;
            continue;
        };
        let key = assemble::identity(&fields, &item.raw);
        if store.holds(account.id.clone(), &key)? {
            total.already += 1;
            continue;
        }
        let raw = store.blobs().put(&item.raw)?;
        if waiting.contains(&raw) {
            total.already += 1;
            continue;
        }
        store.enqueue(
            account.id.clone(),
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
        )?;
        waiting.push(raw);
        total.added += 1;
        if total.read % BATCH == 0 {
            progress(&total);
        }
    }
    progress(&total);
    Ok((account.id, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{MailboxRole, ReadState, Star};

    fn message(id: &str) -> String {
        format!(
            "Message-ID: <{id}@example.test>\nFrom: a@example.test\nTo: b@example.test\n\
             Subject: {id}\nDate: Tue, 07 Nov 2023 13:20:51 +0000\n\nbody\n"
        )
    }

    fn mbox_of(messages: &[(&str, Option<&str>)]) -> String {
        messages
            .iter()
            .map(|(id, status)| {
                let status = status
                    .map(|s| format!("X-Mozilla-Status: {s}\n"))
                    .unwrap_or_default();
                format!("From - Tue Nov 07 13:20:51 2023\n{status}{}\n", message(id))
            })
            .collect()
    }

    fn emlx_of(id: &str, flags: u64) -> Vec<u8> {
        let raw = message(id);
        format!(
            "{:<10}\n{raw}<?xml version=\"1.0\"?>\n<plist version=\"1.0\"><dict>\
             <key>date-received</key><integer>1699363251</integer>\
             <key>flags</key><integer>{flags}</integer></dict></plist>\n",
            raw.len()
        )
        .into_bytes()
    }

    fn put(path: &Path, bytes: impl AsRef<[u8]>) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Each message's subject, with its role, labels, read state and star.
    fn read(source: &Source) -> Vec<(String, MailboxRole, Vec<String>, ReadState, Star)> {
        source
            .items()
            .unwrap()
            .map(|item| {
                let item = item.unwrap();
                let subject = archive::header(&item.raw, "Subject").unwrap();
                let placed = item.placement;
                (
                    subject,
                    placed.role,
                    placed.labels.clone(),
                    placed.read(),
                    placed.star(),
                )
            })
            .collect()
    }

    #[test]
    fn an_apple_mail_export_is_its_packages_named_by_their_mailboxes() {
        let dir = tempfile::tempdir().unwrap();
        let export = dir.path().join("Mail export");
        put(
            &export.join("Sent Messages.mbox/mbox"),
            mbox_of(&[("sent", None)]),
        );
        put(
            &export.join("Sent Messages.mbox/table_of_contents"),
            b"\0\x01",
        );
        put(&export.join("Work.mbox/mbox"), mbox_of(&[("work", None)]));
        put(
            &export.join("Work.mbox/2019.mbox/mbox"),
            mbox_of(&[("old", None)]),
        );
        let source = detect(&export).unwrap();
        assert_eq!(source, Source::new(&AppleMail, export.clone()));
        let archived = |id: &str, label: &str| {
            (
                id.to_owned(),
                MailboxRole::Archive,
                vec![label.to_owned()],
                ReadState::Read,
                Star::Unstarred,
            )
        };
        assert_eq!(
            read(&source),
            vec![
                (
                    "sent".to_owned(),
                    MailboxRole::Sent,
                    Vec::new(),
                    ReadState::Read,
                    Star::Unstarred
                ),
                archived("old", "Work/2019"),
                archived("work", "Work"),
            ]
        );

        // One package chosen on its own, and the mbox inside it, are the same mailbox.
        let package = export.join("Work.mbox");
        assert_eq!(
            detect(&package).unwrap(),
            Source::new(&AppleMail, package.clone())
        );
        assert_eq!(
            read(&Source::new(&AppleMail, package.clone()))[1],
            archived("work", "Work")
        );
        let inner = package.join("mbox");
        assert_eq!(detect(&inner).unwrap(), Source::new(&Mbox, inner.clone()));
        assert_eq!(
            read(&Source::new(&Mbox, inner)),
            vec![archived("work", "Work")]
        );
    }

    #[test]
    fn a_package_reached_by_dot_dot_still_names_its_mailbox() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("Work.mbox");
        put(&package.join("mbox"), mbox_of(&[("work", None)]));
        put(&package.join("Inner/keep"), b"");
        let by_dots = package.join("Inner").join("..");
        let source = detect(&by_dots).unwrap();
        assert_eq!(source, Source::new(&AppleMail, by_dots));
        assert_eq!(read(&source)[0].2, vec!["Work".to_owned()]);
    }

    #[test]
    fn apple_mails_library_is_read_from_its_emlx_files_with_their_flags() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("V10");
        let messages = library.join("ACCOUNT-ID/INBOX.mbox/STORE-ID/Data/1/Messages");
        // Read and flagged; unread; read and deleted.
        put(&messages.join("10.emlx"), emlx_of("starred", 0b1_0001));
        put(&messages.join("11.partial.emlx"), emlx_of("unread", 0));
        put(&messages.join("12.emlx"), emlx_of("deleted", 0b11));
        put(
            &library.join("ACCOUNT-ID/INBOX.mbox/Info.plist"),
            b"<plist/>",
        );
        let source = detect(&library).unwrap();
        assert_eq!(source, Source::new(&AppleMail, library.clone()));
        assert_eq!(
            read(&source),
            vec![
                (
                    "starred".to_owned(),
                    MailboxRole::Inbox,
                    Vec::new(),
                    ReadState::Read,
                    Star::Starred
                ),
                (
                    "unread".to_owned(),
                    MailboxRole::Inbox,
                    Vec::new(),
                    ReadState::Unread,
                    Star::Unstarred
                ),
                (
                    "deleted".to_owned(),
                    MailboxRole::Trash,
                    Vec::new(),
                    ReadState::Read,
                    Star::Unstarred
                ),
            ]
        );

        let one = messages.join("10.emlx");
        let source = detect(&one).unwrap();
        assert_eq!(source, Source::new(&Emlx, one.clone()));
        let item = source.items().unwrap().next().unwrap().unwrap();
        assert_eq!(item.received, DateTime::from_timestamp(1_699_363_251, 0));
        assert!(item.raw.ends_with(b"body\r\n"), "only the message, in CRLF");

        let broken = dir.path().join("broken.emlx");
        put(&broken, b"From: a@b\n\nnot an emlx");
        let refused = Source::new(&Emlx, broken).items().unwrap().next().unwrap();
        assert!(matches!(refused, Err(CoreError::NotEmlx(_))), "{refused:?}");
    }

    #[test]
    fn a_thunderbird_profile_is_every_accounts_folders_without_what_was_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("abcd1234.default-release");
        put(&profile.join("prefs.js"), b"user_pref(\"x\", 1);\n");
        let local = profile.join("Mail/Local Folders");
        put(
            &local.join("Inbox"),
            mbox_of(&[("kept", Some("0001")), ("gone", Some("0009"))]),
        );
        put(
            &local.join("Inbox.msf"),
            b"// <!-- <mdb:mork:z v=\"1.4\"/> -->",
        );
        put(&local.join("msgFilterRules.dat"), b"version=\"9\"\n");
        put(&local.join("Trash"), b"");
        put(
            &local.join("Projects.sbd/Mailo"),
            mbox_of(&[("nested", Some("0004"))]),
        );
        put(
            &profile.join("ImapMail/imap.example.test/Sent"),
            mbox_of(&[("sent", Some("0003"))]),
        );
        put(&profile.join("ImapMail/imap.example.test/Sent.msf"), b"");

        let source = detect(&profile).unwrap();
        assert_eq!(source, Source::new(&Thunderbird, profile.clone()));
        assert_eq!(
            read(&source),
            vec![
                (
                    "kept".to_owned(),
                    MailboxRole::Inbox,
                    Vec::new(),
                    ReadState::Read,
                    Star::Unstarred
                ),
                (
                    "nested".to_owned(),
                    MailboxRole::Archive,
                    vec!["Projects/Mailo".to_owned()],
                    ReadState::Unread,
                    Star::Starred
                ),
                (
                    "sent".to_owned(),
                    MailboxRole::Sent,
                    Vec::new(),
                    ReadState::Read,
                    Star::Unstarred
                ),
            ]
        );

        // `Mail` itself, and one account's directory, are Thunderbird's too.
        for chosen in [profile.join("Mail"), local.clone()] {
            assert_eq!(
                detect(&chosen).unwrap(),
                Source::new(&Thunderbird, chosen.clone())
            );
            assert_eq!(read(&Source::new(&Thunderbird, chosen)).len(), 2);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_subfolder_that_cannot_be_listed_stops_the_import_naming_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let account = dir.path().join("Mail/Local Folders");
        put(&account.join("Inbox"), mbox_of(&[("kept", None)]));
        put(&account.join("Inbox.msf"), b"");
        let locked = account.join("Projects.sbd");
        put(&locked.join("Mailo"), mbox_of(&[("nested", None)]));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let refused = Source::new(&Thunderbird, account.clone()).items();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Running as a user who can list it anyway proves nothing.
        if let Err(e) = refused {
            assert!(e.to_string().contains("the folder Projects"), "{e}");
        }
    }

    #[test]
    fn an_imap_accounts_folder_names_are_read_from_modified_utf7() {
        let dir = tempfile::tempdir().unwrap();
        let account = dir.path().join("ImapMail/imap.example.test");
        put(&account.join("Entw&APw-rfe"), mbox_of(&[("draft", None)]));
        put(&account.join("Entw&APw-rfe.msf"), b"");
        put(
            &account.join("Entw&APw-rfe.sbd/Alt&AOQ-"),
            mbox_of(&[("old", None)]),
        );
        let labels: Vec<Vec<String>> = read(&Source::new(&Thunderbird, dir.path().to_owned()))
            .into_iter()
            .map(|message| message.2)
            .collect();
        assert!(labels.contains(&vec!["Entwürfe".to_owned()]), "{labels:?}");
        assert!(
            labels.contains(&vec!["Entwürfe/Altä".to_owned()]),
            "{labels:?}"
        );
        // A local folder's name is its own: `&` in it is an ampersand.
        let local = dir.path().join("Mail/Local Folders");
        put(&local.join("R&D-x"), mbox_of(&[("rd", None)]));
        put(&local.join("R&D-x.msf"), b"");
        let all = read(&Source::new(&Thunderbird, local));
        assert_eq!(all[0].2, vec!["R&D-x".to_owned()]);
    }

    #[test]
    fn a_message_cut_off_and_a_file_without_attachments_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let account = dir.path().join("ImapMail/imap.example.test");
        put(
            &account.join("INBOX"),
            mbox_of(&[("whole", Some("0001")), ("cut", Some("0401"))]),
        );
        put(&account.join("INBOX.msf"), b"");
        let mut total = Imported::default();
        let kept: Vec<bool> = Source::new(&Thunderbird, dir.path().to_owned())
            .items()
            .unwrap()
            .map(|item| item.unwrap().tally(&mut total))
            .collect();
        assert_eq!(kept, [true, false]);
        assert_eq!((total.read, total.cut_off, total.partial), (2, 1, 0));

        let messages = dir.path().join("Messages");
        put(&messages.join("1.partial.emlx"), emlx_of("a", 0));
        put(&messages.join("2.emlx"), emlx_of("b", 0));
        let mut total = Imported::default();
        for item in Source::new(&AppleMail, dir.path().join("Messages"))
            .items()
            .unwrap()
        {
            assert!(item.unwrap().tally(&mut total));
        }
        assert_eq!((total.read, total.partial, total.cut_off), (2, 1, 0));
    }

    #[test]
    fn a_directory_of_something_else_is_still_refused() {
        let dir = tempfile::tempdir().unwrap();
        put(&dir.path().join("Photos/one.jpg"), b"\xff\xd8");
        let refused = detect(&dir.path().join("Photos"));
        assert!(
            matches!(refused, Err(CoreError::NotMailFolder(_))),
            "{refused:?}"
        );
    }
}
