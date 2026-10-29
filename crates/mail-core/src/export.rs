//! `mailo export`: messages out to files — an mbox, a Maildir, or a directory of `.eml`.
//!
//! Which messages is said the way `mailo search` says it, with the same parser and expansion
//! ([`crate::search::prepare`]), or by a place's name: `inbox`, `sent`, `all`. The
//! bytes written are the raw message as the server sent it, never a rendering.
//!
//! A message whose body has not been downloaded yet has no bytes to write, and neither does one
//! whose attachments were left on the server; both are skipped and counted rather than fetched
//! here, because a sync already knows how to fetch them and an export that silently opened
//! connections halfway through would be two commands in one. The report says how many and that
//! `mailo sync` fills them in.

use crate::environment::Environment;
use crate::error::{CoreError, Logged};
use crate::formats::{Ctx, Sink, Writer};
use chrono::{DateTime, Utc};
use mail_domain::{
    Body, Filter, LabelId, MailboxRole, MatchCtx, Message, MessageId, Mute, PageReq, Pin, Property,
    Query, Snooze, Sort, SortDir, SystemFlag, ThreadSummary,
};
use mail_mime::archive::{maildir, mbox};
use mail_store::{SqliteStore, Store};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the messages go: a format, and the file or directory it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub writer: &'static dyn Writer,
    pub path: PathBuf,
}

impl Target {
    pub fn new(writer: &'static dyn Writer, path: PathBuf) -> Self {
        Self { writer, path }
    }
}

/// The format an export is written in unless another is chosen: an mbox.
pub fn usual() -> &'static dyn Writer {
    &MboxWriter
}

/// What an export did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exported {
    pub written: usize,
    /// Only headers are here yet: the body was never downloaded.
    pub absent: usize,
    /// The body is here, but it was rebuilt from its parts with some attachment left on the
    /// server, so the stored bytes are not the message as sent.
    pub partial: usize,
}

/// How many threads are asked for at a time while choosing messages.
const PAGE: u32 = 200;

/// The messages `query` means, oldest first.
///
/// A place's name alone — `inbox`, `archive`, `sent`, `drafts`, `trash`, `spam` — means the
/// messages in it, and `all` means every message. Anything else is a search, and each thread
/// it finds contributes the messages that match it on their own; where none does, because the
/// terms were met by different messages of the thread, the whole thread is taken, since that
/// is what the search found.
pub fn select(
    store: &SqliteStore,
    query: &str,
    now: DateTime<Utc>,
) -> Result<Vec<MessageId>, CoreError> {
    let words = query.trim();
    let place = place_named(words);
    let (filter, regex) = match (place, words) {
        (Some(Chosen::Role(role)), _) => (Filter::InMailbox(role), None),
        (Some(Chosen::All), _) => (Filter::All, None),
        (None, "") => return Err(CoreError::NoSelection),
        (None, _) => {
            let index = crate::query::known_labels(store);
            let label = crate::query::named(&index);
            let prepared = crate::search::prepare(words, store, &chrono::Local, &label)?;
            (prepared.filter, prepared.regex)
        }
    };

    let mut chosen: Vec<(DateTime<Utc>, MessageId)> = Vec::new();
    let mut after = None;
    loop {
        let page = store.threads(
            &Query {
                filter: filter.clone(),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: after.clone(),
                    limit: PAGE,
                },
            },
            now,
        )?;
        for summary in &page.items {
            if let Some(regex) = &regex
                && !crate::search::regex_hits(regex, &summary.subject, &summary.snippet)
            {
                continue;
            }
            let thread = store.thread(summary.id)?;
            let messages: Vec<Message> = thread
                .messages
                .iter()
                .filter_map(|id| store.message(*id).or_log("a message could not be read"))
                .collect();
            let fits: Vec<&Message> = match place {
                Some(Chosen::Role(role)) => messages.iter().filter(|m| m.mailbox == role).collect(),
                Some(Chosen::All) => messages.iter().collect(),
                None => {
                    let alone: Vec<&Message> = messages
                        .iter()
                        .filter(|m| fits_alone(store, &filter, m, now))
                        .collect();
                    if alone.is_empty() {
                        messages.iter().collect()
                    } else {
                        alone
                    }
                }
            };
            chosen.extend(fits.into_iter().map(|m| (m.date, m.id)));
        }
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    chosen.sort();
    chosen.dedup_by_key(|(_, id)| *id);
    Ok(chosen.into_iter().map(|(_, id)| id).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chosen {
    Role(MailboxRole),
    All,
}

fn place_named(words: &str) -> Option<Chosen> {
    // Checked before the role, whose parser reads `all` as the archive.
    if words.eq_ignore_ascii_case("all") || words.eq_ignore_ascii_case("everything") {
        return Some(Chosen::All);
    }
    // Words that name no role are not a choice.
    words.parse().ok().map(Chosen::Role)
}

/// Whether one message on its own is what `filter` asks for.
///
/// Its own server addresses, for `Filter::InFolder`: a copy of the thread elsewhere does not
/// put this message in that folder.
fn fits_alone(store: &SqliteStore, filter: &Filter, message: &Message, now: DateTime<Utc>) -> bool {
    let folders = store
        .placed(message.id)
        .or_log_default("the folders a message is in could not be read");
    let summary = ThreadSummary::derive(
        message.thread,
        std::slice::from_ref(message),
        Snooze::Inactive,
        Pin::Unpinned,
        Mute::Unmuted,
        mail_domain::FollowUp::Inactive,
    );
    let corpus = match &message.body {
        Body::Present { text, .. } => text.as_deref(),
        Body::Absent => None,
    };
    let sizes = [mail_domain::filter::message_size(message)];
    let vips: Vec<String> = store
        .vips()
        .or_log_default("the VIPs could not be read")
        .into_iter()
        .map(|vip| vip.address)
        .collect();
    filter.fit(&MatchCtx {
        summary: &summary,
        corpus,
        folders: &folders,
        sizes: &sizes,
        cc: &message.cc,
        vips: &vips,
        now,
    })
}

/// Write `messages` to `target`, calling `progress` every hundred.
pub fn export(
    store: &SqliteStore,
    env: &Environment,
    messages: &[MessageId],
    target: &Target,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Exported),
) -> Result<Exported, CoreError> {
    let ctx = Ctx { store, env, now };
    let mut sink = target.writer.open(&target.path, &ctx)?;
    let mut done = Exported::default();
    for id in messages {
        let message = store.message(*id)?;
        let Body::Present { raw, .. } = &message.body else {
            done.absent += 1;
            continue;
        };
        let bytes = store.blobs().get(*raw)?;
        // Read from the bytes as well as the attachments: a part fetched since leaves the
        // attachment held and the stored message as rebuilt, its part still empty in it.
        if crate::compose::rebuilt(&message, &bytes) {
            done.partial += 1;
            continue;
        }
        sink.write(&message, &bytes)?;
        done.written += 1;
        if done.written % 100 == 0 {
            progress(&done);
        }
    }
    sink.finish()?;
    Ok(done)
}

/// `id`'s stored bytes: the message as the server sent it, never a rendering. `None` while only
/// its headers are here.
///
/// A message rebuilt from its parts ([`crate::compose::rebuilt`]) gives the rebuilt bytes, which
/// are what the reader draws it from.
pub fn raw_message(store: &SqliteStore, id: MessageId) -> Result<Option<Vec<u8>>, CoreError> {
    let message = store.message(id)?;
    raw_of(store, &message)
}

/// The first bytes of a message's source, and how long the whole of it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHead {
    /// At most the `limit` asked for.
    pub bytes: Vec<u8>,
    /// The size of the whole source.
    pub whole: u64,
}

/// At most the first `limit` bytes of `id`'s [`raw_message`], and its whole size: what a view of
/// a message's source needs, without reading a large attachment in full. `None` while only its
/// headers are here.
pub fn raw_head(
    store: &SqliteStore,
    id: MessageId,
    limit: usize,
) -> Result<Option<RawHead>, CoreError> {
    let message = store.message(id)?;
    let Body::Present { raw, .. } = &message.body else {
        return Ok(None);
    };
    Ok(Some(RawHead {
        bytes: store.blobs().head(*raw, limit)?,
        whole: store.blobs().size(*raw)?,
    }))
}

/// Write `id`'s [`raw_message`] into `dir` as one `.eml` named from its subject, never over a
/// file already there, returning the path written. `None`, and nothing written, while only its
/// headers are here.
pub fn save_eml(
    store: &SqliteStore,
    id: MessageId,
    dir: &Path,
) -> Result<Option<PathBuf>, CoreError> {
    let message = store.message(id)?;
    let Some(bytes) = raw_of(store, &message)? else {
        return Ok(None);
    };
    let name = crate::compose::eml_name(&message.subject);
    crate::attach::write_new(dir, &name, &bytes).map(Some)
}

fn raw_of(store: &SqliteStore, message: &Message) -> Result<Option<Vec<u8>>, CoreError> {
    match &message.body {
        Body::Present { raw, .. } => Ok(Some(store.blobs().get(*raw)?)),
        Body::Absent => Ok(None),
    }
}

/// One mbox file, mboxrd-quoted. Refused if the file exists.
#[derive(Debug)]
pub struct MboxWriter;

impl Writer for MboxWriter {
    fn slug(&self) -> &'static str {
        "mbox"
    }

    fn label(&self) -> &'static str {
        "Mbox"
    }

    fn suggested(&self, stem: &str) -> String {
        format!("mailo-{stem}.mbox")
    }

    fn open(&self, path: &Path, _ctx: &Ctx<'_>) -> Result<Box<dyn Sink>, CoreError> {
        // Never over an existing file: an export that replaced someone's archive with a
        // search that matched three messages would be the worst kind of success.
        let file = create_private(path).map_err(|source| CoreError::NotCreated {
            path: path.to_owned(),
            source,
        })?;
        Ok(Box::new(MboxSink(std::io::BufWriter::new(file))))
    }
}

struct MboxSink(std::io::BufWriter<std::fs::File>);

impl Sink for MboxSink {
    fn write(&mut self, message: &Message, raw: &[u8]) -> Result<(), CoreError> {
        mbox::write(
            &mut self.0,
            &mbox::envelope_of(raw, Some(message.date)),
            raw,
        )
        .map_err(CoreError::from)
    }

    fn finish(mut self: Box<Self>) -> Result<(), CoreError> {
        self.0.flush().map_err(CoreError::from)
    }
}

/// A Maildir, created if absent; roles and labels become Maildir++ folders.
#[derive(Debug)]
pub struct MaildirWriter;

impl Writer for MaildirWriter {
    fn slug(&self) -> &'static str {
        "maildir"
    }

    fn label(&self) -> &'static str {
        "Maildir"
    }

    fn suggested(&self, stem: &str) -> String {
        format!("mailo-{stem}")
    }

    fn open(&self, path: &Path, ctx: &Ctx<'_>) -> Result<Box<dyn Sink>, CoreError> {
        make_maildir(path)?;
        Ok(Box::new(MaildirSink {
            root: path.to_owned(),
            labels: crate::query::known_labels(ctx.store),
            now: ctx.now,
            count: 0,
            host: host(ctx.env),
        }))
    }
}

struct MaildirSink {
    root: PathBuf,
    labels: Vec<(String, LabelId)>,
    now: DateTime<Utc>,
    count: u64,
    /// This machine's name, in each file's unique name.
    host: String,
}

impl Sink for MaildirSink {
    fn write(&mut self, message: &Message, raw: &[u8]) -> Result<(), CoreError> {
        self.count += 1;
        let dir = match folder_of(message, &self.labels) {
            Some(folder) => {
                let dir = self.root.join(maildir::dir_of_folder(&folder));
                make_maildir(&dir)?;
                // Maildir++ marks a folder with this empty file.
                let marker = dir.join("maildirfolder");
                if !marker.exists() {
                    create_private(&marker).map_err(CoreError::at(&marker))?;
                }
                dir
            }
            None => self.root.clone(),
        };
        let unique = maildir::unique(&maildir::Unique {
            secs: self.now.timestamp(),
            micros: self.now.timestamp_subsec_micros(),
            pid: std::process::id(),
            count: self.count,
            host: self.host.as_str(),
        });
        let name = maildir::name(&unique, &maildir::flags_of(&flags_of(message)));
        // Written in `tmp` and renamed into `cur`, as the spec says: a reader never sees
        // half a message under a finished name.
        let tmp = dir.join("tmp").join(&unique);
        create_private(&tmp)
            .and_then(|mut out| out.write_all(&mail_mime::archive::lf(raw)))
            .map_err(CoreError::at(&tmp))?;
        let cur = dir.join("cur").join(&name);
        std::fs::rename(&tmp, &cur).map_err(CoreError::at(&cur))
    }
}

/// One `.eml` per message in a directory, created if absent.
#[derive(Debug)]
pub struct EmlWriter;

impl Writer for EmlWriter {
    fn slug(&self) -> &'static str {
        "eml"
    }

    fn label(&self) -> &'static str {
        ".eml"
    }

    fn suggested(&self, stem: &str) -> String {
        format!("mailo-{stem}-eml")
    }

    fn open(&self, path: &Path, _ctx: &Ctx<'_>) -> Result<Box<dyn Sink>, CoreError> {
        std::fs::create_dir_all(path).map_err(CoreError::at(path))?;
        Ok(Box::new(EmlSink(path.to_owned())))
    }
}

struct EmlSink(PathBuf);

impl Sink for EmlSink {
    fn write(&mut self, message: &Message, raw: &[u8]) -> Result<(), CoreError> {
        let id = message.id.to_string();
        let file = self.0.join(format!(
            "{}-{}.eml",
            message.date.format("%Y%m%d-%H%M%S"),
            &id[..8.min(id.len())]
        ));
        let mut out = create_private(&file).map_err(CoreError::at(&file))?;
        out.write_all(raw).map_err(CoreError::at(&file))
    }
}

/// A new file at `path` that only its owner can read (0600), as an attachment saved to disk is:
/// an export is someone's mail, and the umask would leave it readable by everyone on the machine.
/// Never over an existing file.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// The folder a message is written to: its role's, or for archived mail its first label, so
/// that importing the Maildir again gives it the same label back.
fn folder_of(message: &Message, labels: &[(String, LabelId)]) -> Option<String> {
    if message.mailbox == MailboxRole::Archive
        && let Some(name) = message
            .labels
            .iter()
            .find_map(|id| labels.iter().find(|(_, l)| l == id).map(|(n, _)| n.clone()))
    {
        return Some(name);
    }
    maildir::folder_for_role(message.mailbox).map(str::to_owned)
}

fn flags_of(message: &Message) -> Vec<SystemFlag> {
    let mut flags = Vec::new();
    if message.read == mail_domain::ReadState::Read {
        flags.push(SystemFlag::Seen);
    }
    if message.star == mail_domain::Star::Starred {
        flags.push(SystemFlag::Flagged);
    }
    if message.mailbox == MailboxRole::Drafts {
        flags.push(SystemFlag::Draft);
    }
    flags
}

/// `cur`, `new` and `tmp` under `dir`.
fn make_maildir(dir: &Path) -> Result<(), CoreError> {
    for sub in ["cur", "new", "tmp"] {
        let path = dir.join(sub);
        std::fs::create_dir_all(&path).map_err(CoreError::at(&path))?;
    }
    Ok(())
}

/// This machine's name for a Maildir unique name, from the environment where it says.
fn host(env: &Environment) -> String {
    env.hostname
        .clone()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_owned())
        })
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".to_owned())
}
