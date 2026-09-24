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

use chrono::{DateTime, Utc};
use mail_domain::{
    Body, Filter, LabelId, MailboxRole, MatchCtx, Message, MessageId, PageReq, PartContent, Pin,
    Property, Query, Snooze, Sort, SortDir, SystemFlag, ThreadSummary,
};
use mail_mime::archive::{maildir, mbox};
use mail_store::{SqliteStore, Store};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the messages go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// One mbox file, mboxrd-quoted. Refused if the file exists.
    Mbox(PathBuf),
    /// A Maildir, created if absent; roles and labels become Maildir++ folders.
    Maildir(PathBuf),
    /// One `.eml` per message in a directory, created if absent.
    Eml(PathBuf),
}

/// What an export did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exported {
    pub written: usize,
    /// Only headers are here yet: the body was never downloaded.
    pub absent: usize,
    /// The body is here, but some attachment was left on the server.
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
) -> Result<Vec<MessageId>, String> {
    let words = query.trim();
    let place = place_named(words);
    let (filter, regex) = match (place, words) {
        (Some(Chosen::Role(role)), _) => (Filter::InMailbox(role), None),
        (Some(Chosen::All), _) => (Filter::All, None),
        (None, "") => return Err("say which messages: a search, or inbox, sent, all…".to_owned()),
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
        let page = store
            .threads(
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
            )
            .map_err(|e| e.to_string())?;
        for summary in &page.items {
            if let Some(regex) = &regex
                && !crate::search::regex_hits(regex, &summary.subject, &summary.snippet)
            {
                continue;
            }
            let thread = store.thread(summary.id).map_err(|e| e.to_string())?;
            let messages: Vec<Message> = thread
                .messages
                .iter()
                .filter_map(|id| store.message(*id).ok())
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
    Some(match words.to_ascii_lowercase().as_str() {
        "inbox" => Chosen::Role(MailboxRole::Inbox),
        "archive" => Chosen::Role(MailboxRole::Archive),
        "sent" => Chosen::Role(MailboxRole::Sent),
        "drafts" => Chosen::Role(MailboxRole::Drafts),
        "trash" => Chosen::Role(MailboxRole::Trash),
        "spam" => Chosen::Role(MailboxRole::Spam),
        "all" | "everything" => Chosen::All,
        _ => return None,
    })
}

/// Whether one message on its own is what `filter` asks for.
///
/// Its own server addresses, for `Filter::InFolder`: a copy of the thread elsewhere does not
/// put this message in that folder.
fn fits_alone(store: &SqliteStore, filter: &Filter, message: &Message, now: DateTime<Utc>) -> bool {
    let folders = store.placed(message.id).unwrap_or_default();
    let summary = ThreadSummary::derive(
        message.thread,
        std::slice::from_ref(message),
        Snooze::Inactive,
        Pin::Unpinned,
    );
    let corpus = match &message.body {
        Body::Present { text, .. } => text.as_deref(),
        Body::Absent => None,
    };
    filter.fit(&MatchCtx {
        summary: &summary,
        corpus,
        folders: &folders,
        now,
    })
}

/// Write `messages` to `target`, calling `progress` every hundred.
pub fn export(
    store: &SqliteStore,
    messages: &[MessageId],
    target: &Target,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Exported),
) -> Result<Exported, String> {
    let mut sink = Sink::open(store, target, now)?;
    let mut done = Exported::default();
    for id in messages {
        let message = store.message(*id).map_err(|e| e.to_string())?;
        let Body::Present { raw, .. } = &message.body else {
            done.absent += 1;
            continue;
        };
        if message
            .attachments
            .iter()
            .any(|a| matches!(a.content, PartContent::Remote { .. }))
        {
            done.partial += 1;
            continue;
        }
        let bytes = store
            .blobs()
            .get(&store.connection(), *raw)
            .map_err(|e| e.to_string())?;
        sink.write(&message, &bytes)?;
        done.written += 1;
        if done.written % 100 == 0 {
            progress(&done);
        }
    }
    sink.finish()?;
    Ok(done)
}

/// What an export says when it is done.
pub fn said(done: &Exported, target: &Target) -> String {
    let place = match target {
        Target::Mbox(path) | Target::Maildir(path) | Target::Eml(path) => path.display(),
    };
    let mut out = format!("{} message(s) written to {place}\n", done.written);
    let skipped = done.absent + done.partial;
    if skipped > 0 {
        out.push_str(&format!(
            "{skipped} skipped: {} with no body downloaded yet, {} with attachments still on \
             the server. `mailo sync` downloads bodies; `mailo save` fetches an attachment.\n",
            done.absent, done.partial
        ));
    }
    out
}

/// An open destination.
enum Sink {
    Mbox(std::io::BufWriter<std::fs::File>),
    Maildir {
        root: PathBuf,
        labels: Vec<(String, LabelId)>,
        now: DateTime<Utc>,
        count: u64,
    },
    Eml(PathBuf),
}

impl Sink {
    fn open(store: &SqliteStore, target: &Target, now: DateTime<Utc>) -> Result<Self, String> {
        match target {
            Target::Mbox(path) => {
                // Never over an existing file: an export that replaced someone's archive with a
                // search that matched three messages would be the worst kind of success.
                let file = std::fs::File::create_new(path).map_err(|e| {
                    format!(
                        "{}: {e}; choose a name that does not exist yet",
                        path.display()
                    )
                })?;
                Ok(Sink::Mbox(std::io::BufWriter::new(file)))
            }
            Target::Maildir(root) => {
                make_maildir(root)?;
                Ok(Sink::Maildir {
                    root: root.clone(),
                    labels: crate::query::known_labels(store),
                    now,
                    count: 0,
                })
            }
            Target::Eml(dir) => {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                Ok(Sink::Eml(dir.clone()))
            }
        }
    }

    fn write(&mut self, message: &Message, raw: &[u8]) -> Result<(), String> {
        match self {
            Sink::Mbox(out) => mbox::write(out, &mbox::envelope_of(raw, Some(message.date)), raw)
                .map_err(|e| e.to_string()),
            Sink::Maildir {
                root,
                labels,
                now,
                count,
            } => {
                *count += 1;
                let dir = match folder_of(message, labels) {
                    Some(folder) => {
                        let dir = root.join(maildir::dir_of_folder(&folder));
                        make_maildir(&dir)?;
                        // Maildir++ marks a folder with this empty file.
                        let marker = dir.join("maildirfolder");
                        if !marker.exists() {
                            std::fs::write(&marker, b"")
                                .map_err(|e| format!("{}: {e}", marker.display()))?;
                        }
                        dir
                    }
                    None => root.clone(),
                };
                let unique = maildir::unique(&maildir::Unique {
                    secs: now.timestamp(),
                    micros: now.timestamp_subsec_micros(),
                    pid: std::process::id(),
                    count: *count,
                    host: &host(),
                });
                let name = maildir::name(&unique, &maildir::flags_of(&flags_of(message)));
                // Written in `tmp` and renamed into `cur`, as the spec says: a reader never sees
                // half a message under a finished name.
                let tmp = dir.join("tmp").join(&unique);
                std::fs::write(&tmp, mail_mime::archive::lf(raw))
                    .map_err(|e| format!("{}: {e}", tmp.display()))?;
                let cur = dir.join("cur").join(&name);
                std::fs::rename(&tmp, &cur).map_err(|e| format!("{}: {e}", cur.display()))
            }
            Sink::Eml(dir) => {
                let id = message.id.to_string();
                let file = dir.join(format!(
                    "{}-{}.eml",
                    message.date.format("%Y%m%d-%H%M%S"),
                    &id[..8.min(id.len())]
                ));
                let mut out = std::fs::File::create_new(&file)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                out.write_all(raw)
                    .map_err(|e| format!("{}: {e}", file.display()))
            }
        }
    }

    fn finish(self) -> Result<(), String> {
        match self {
            Sink::Mbox(mut out) => out.flush().map_err(|e| e.to_string()),
            Sink::Maildir { .. } | Sink::Eml(_) => Ok(()),
        }
    }
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
fn make_maildir(dir: &Path) -> Result<(), String> {
    for sub in ["cur", "new", "tmp"] {
        let path = dir.join(sub);
        std::fs::create_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// This machine's name for a Maildir unique name, from the environment where it says.
fn host() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_owned())
        })
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".to_owned())
}
