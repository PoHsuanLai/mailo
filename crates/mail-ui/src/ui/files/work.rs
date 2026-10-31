//! What the Import and Export sheets do, as functions of a client and a path.
//!
//! The sheets only draw what these answer, and the tests drive these rather than a window. The
//! work itself is `mail_client::import` and `mail_client::export`, the same functions `mailo import` and
//! `mailo export` run; this adds the words a person reads while choosing, and the one step the
//! command line takes after an upload — sending it — is [`import_then_send`].

use mail_client::Client;
use std::io::ErrorKind;
use std::path::Path;

use chrono::{DateTime, Utc};
use mail_client::mailbox_filter::{for_role, pending_snooze};
use mail_client::transfer::{self, IoCause, Refusal, Seen};
use mail_domain::{Filter, MailboxRole, Star};
use porter_core::AccountId;

use mail_client::export::{Exported, Target};
use mail_client::import::{Destination, Imported, Source};
use mail_ui_kit::view::{Shell, Source as Listed};
use mail_ui_model::copy;

pub(in crate::ui) use mail_client::transfer::{expand_here, suggested};

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
    match transfer::look(path) {
        Seen::Blank => Looked::Blank,
        Seen::Mail { source, count } => {
            let said = described(&source, count);
            Looked::Mail {
                source,
                count,
                said,
            }
        }
        Seen::Refused(why) => Looked::Refused(refused(path, &why)),
    }
}

/// What the sheet says of a source it found: its kind and message count, and for a Thunderbird
/// profile that only what Thunderbird kept offline can be imported.
pub(in crate::ui) fn described(source: &Source, count: usize) -> String {
    let slug = source.archive.slug();
    let said = format!("{}, {}", copy::status::source_kind(slug), messages(count));
    match copy::status::source_note(slug) {
        Some(note) => format!("{said}. {note}"),
        None => said,
    }
}

/// Why a path cannot be imported, in words.
fn refused(path: &Path, why: &Refusal) -> String {
    let shown = path.display();
    match why {
        Refusal::NothingThere => format!("Nothing at {shown}."),
        Refusal::CannotReach(cause) => {
            format!("Can’t reach {shown}: {}.", plain(cause))
        }
        Refusal::CannotOpen(cause) => format!("Can’t open {shown}: {}.", plain(cause)),
        Refusal::Empty => format!("{shown} has no mail in it."),
        Refusal::Unreadable(said) => sentence(said),
    }
}

/// An I/O error as a person reads it: no `os error 13`.
fn plain(cause: &IoCause) -> String {
    match cause.kind {
        ErrorKind::PermissionDenied => "you don’t have permission to read it".to_owned(),
        ErrorKind::NotFound => "it isn’t there".to_owned(),
        _ => match cause.text.split_once(" (os error") {
            Some((words, _)) => words.to_lowercase(),
            None => cause.text.clone(),
        },
    }
}

/// A reason from `import` as a sentence: a capital letter and a full stop.
fn sentence(why: &dyn std::fmt::Display) -> String {
    let said = why.to_string();
    let why = said.trim().trim_end_matches('.');
    let mut chars = why.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => "Can’t read that.".to_owned(),
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
/// the client lists them, accounts oldest first. POP3 has no folders to upload into.
pub(in crate::ui) fn destinations(client: &Client) -> Vec<Dest> {
    std::iter::once(Dest::Local)
        .chain(
            client
                .transfer()
                .upload_folders()
                .into_iter()
                .map(|upload| Dest::Folder {
                    account: upload.account,
                    address: upload.address,
                    folder: upload.folder,
                }),
        )
        .collect()
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
    client: &Client,
    source: &Source,
    into: &Dest,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<Done, String> {
    let (total, queued) = client
        .transfer()
        .import_into(source, &into.destination(), now, progress)
        .map_err(|why| why.to_string())?;
    let said = mail_os::said::import::said(&total, &into.destination())
        .trim()
        .to_owned();
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
    client: &Client,
    source: &Source,
    into: &Dest,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Imported),
) -> Result<String, String> {
    let done = import_now(client, source, into, now, progress)?;
    let Some(account) = done.queued else {
        return Ok(done.said);
    };
    let sent = match client.net().drain(account) {
        Ok(report) => {
            let mut out = format!("{}. {} uploaded", done.said, report.appended);
            if report.still_queued > 0 {
                out.push_str(&format!(
                    "; {} still queued for the next sync",
                    report.still_queued
                ));
            }
            for note in &report.needs_attention {
                out.push_str(&format!(". Needs attention: {note}"));
            }
            out
        }
        Err(why) => format!(
            "{}. Not uploaded yet: {why}. Queued for the next sync.",
            done.said
        ),
    };
    Ok(sent)
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
        // A saved view in the words it lists by, when there are words for it.
        Listed::Saved(view) => {
            if let Some(words) =
                mail_ui_kit::saved::written(&view.filter, &shell.known(), &chrono::Local)
            {
                return words;
            }
            None
        }
        _ => None,
    };
    if let Some(name) = label {
        return if name.contains(char::is_whitespace) {
            format!("label:\"{name}\"")
        } else {
            format!("label:{name}")
        };
    }
    if let Some(words) = place_words(&place.source) {
        return words.to_owned();
    }
    place.name.to_lowercase()
}

/// A built-in place in the words `export::select` reads, found by what the place lists, not
/// by what it is called. `None` for a folder, whose name is its word.
fn place_words(source: &Listed) -> Option<&'static str> {
    if *source == Listed::Drafts {
        return Some("drafts");
    }
    let Listed::Mail(filter) = source else {
        return None;
    };
    if *filter == Filter::Starred(Star::Starred) {
        return Some("is:starred");
    }
    if *filter == Filter::Pinned {
        return Some("is:pinned");
    }
    if *filter == pending_snooze() {
        return Some("is:snoozed");
    }
    [
        (MailboxRole::Inbox, "inbox"),
        (MailboxRole::Archive, "archive"),
        (MailboxRole::Sent, "sent"),
        (MailboxRole::Trash, "trash"),
        (MailboxRole::Spam, "spam"),
    ]
    .into_iter()
    .find(|(role, _)| *filter == for_role(*role))
    .map(|(_, words)| words)
}

/// How many messages `query` means, as the sheet says it, or why it means none.
pub(in crate::ui) fn counted(client: &Client, query: &str, now: DateTime<Utc>) -> Counted {
    if query.trim().is_empty() {
        return Counted::Blank;
    }
    match client.transfer().selection_size(query, now) {
        Ok(count) => Counted::Some(count),
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
    client: &Client,
    query: &str,
    target: &Target,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Exported),
) -> Result<(Exported, String), String> {
    let chosen = client
        .export()
        .select(query, now)
        .map_err(|why| sentence(&why))?;
    if chosen.is_empty() {
        return Err("Nothing to export.".to_owned());
    }
    let done =
        client
            .export()
            .export(&client.net().environment(), &chosen, target, now, progress)?;
    let said = mail_os::said::export::said(&done, target)
        .lines()
        .collect::<Vec<_>>()
        .join(" ");
    Ok((done, said))
}
