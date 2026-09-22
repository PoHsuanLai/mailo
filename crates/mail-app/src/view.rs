//! What the shell shows, as data.
//!
//! Deliberately free of Dioxus. Which view is selected, what query that means, which thread is
//! open and what the reader should do with a body are all decisions that can be wrong, and none
//! of them needs a window to be wrong in. The rendering layer reads this and draws it.

use chrono::{DateTime, Datelike, TimeDelta, TimeZone, Timelike, Utc, Weekday};
use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};

/// An entry in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    /// What this place lists. Not a `Filter`: see [`Source`].
    pub source: Source,
    /// Shown as a badge. `None` until counted, which is not the same as zero.
    pub unread: Option<u64>,
}

/// The default sidebar.
///
/// Inbox is `Filter::InMailbox(Inbox)` rather than anything special, which is the plan's claim
/// that a place is just a saved filter — made true here rather than asserted.
/// Where a sidebar place gets its rows.
///
/// Two variants because drafts are genuinely not mail yet. A draft has no thread, no server
/// address and no mailbox — it lives in its own table — so there is no `Filter` that selects
/// one, and a `Drafts` place built from `Filter::InMailbox(MailboxRole::Drafts)` lists nothing,
/// for ever, with no error. That was the state of this sidebar until the composer gave it
/// something to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Threads matching a filter.
    Mail(Filter),
    /// The drafts table.
    Drafts,
}

/// The filter a mailbox place lists.
///
/// Public and shared, because there were two of these and they disagreed. The shell asked
/// `source_for`, the CLI's `list` built `Filter::InMailbox(role)` itself, and the day the inbox
/// learned to hide a snoozed conversation only one of them learned it — so `mailo list` went on
/// showing what the window had put away. One definition, both callers.
///
/// `Drafts` is not here: drafts are not threads and no `Filter` selects one. See [`Source`].
pub fn place_filter(role: MailboxRole) -> Filter {
    match role {
        // A snoozed conversation is *away* until its instant passes; that is the whole of what
        // snoozing means, and a client that leaves it in the list has a button that does
        // nothing. Only the inbox hides them — Archive and Search still show everything,
        // because a thread you snoozed is not a thread you lost.
        MailboxRole::Inbox => Filter::And(vec![
            Filter::InMailbox(MailboxRole::Inbox),
            Filter::Not(Box::new(pending_snooze())),
        ]),
        other => Filter::InMailbox(other),
    }
}

fn source_for(role: MailboxRole) -> Source {
    match role {
        MailboxRole::Drafts => Source::Drafts,
        other => Source::Mail(place_filter(other)),
    }
}

/// Snoozed, and not yet due.
///
/// `SnoozeDue` implies `Snoozed`, so "away" is the difference between them rather than a state
/// of its own — which is what `filter.rs` means by predicates rather than state mirrors. Due is
/// resolved against `now` at query time, so a thread returns to the inbox on the stroke without
/// anything having to run.
pub fn pending_snooze() -> Filter {
    Filter::And(vec![
        Filter::Snoozed,
        Filter::Not(Box::new(Filter::SnoozeDue)),
    ])
}

pub fn default_places() -> Vec<Place> {
    let mut places: Vec<Place> = [
        ("Inbox", MailboxRole::Inbox),
        ("Archive", MailboxRole::Archive),
        ("Sent", MailboxRole::Sent),
        ("Drafts", MailboxRole::Drafts),
        ("Spam", MailboxRole::Spam),
        ("Trash", MailboxRole::Trash),
    ]
    .into_iter()
    .map(|(name, role)| Place {
        name: name.to_owned(),
        source: source_for(role),
        unread: None,
    })
    .collect();
    // After Drafts, where a conversation that was put off is looked for: with the other places
    // that hold mail rather than at the end beside Trash.
    let at = places
        .iter()
        .position(|p| p.name == "Drafts")
        .map_or(places.len(), |i| i + 1);
    places.insert(
        at,
        Place {
            name: "Snoozed".to_owned(),
            // Only the ones still away. A due thread is back in the inbox, and showing it here
            // as well would make "snoozed" mean two different things in two places.
            source: Source::Mail(pending_snooze()),
            unread: None,
        },
    );
    places
}

/// What a place's badge counts, or `None` when it has no badge.
///
/// Unread threads, and only for mail: "3 unread drafts" is not a thing, because a draft is not
/// something that arrives and is not something anyone has failed to read yet.
///
/// Sent and Archive get one too. That looks odd until a filter rule files an unread message
/// straight into Archive, at which point a badgeless Archive is a message the user never learns
/// about.
pub fn badge_filter(source: &Source) -> Option<Filter> {
    match source {
        Source::Mail(filter) => Some(Filter::And(vec![
            filter.clone(),
            Filter::Read(ReadState::Unread),
        ])),
        Source::Drafts => None,
    }
}

/// Everything the shell is currently showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub places: Vec<Place>,
    pub selected: usize,
    /// What the user typed in the search box.
    pub search: String,
    pub open: Option<ThreadId>,
    /// Whether the reader may fetch remote images for the thread currently open.
    ///
    /// Per thread and not persisted: consenting to load one sender's images is not consent for
    /// the next message, and a remote image is a read receipt the sender never asked for.
    pub show_remote_images: bool,
    /// The composer, when one is open.
    ///
    /// `Option` rather than a `mode` enum on `Shell`: composing does not replace reading, it
    /// sits beside it. A user who opens a reply and then clicks another thread should still
    /// have their half-written reply when they come back, which a mode would have thrown away.
    pub composing: Option<Composing>,
}

/// A message being edited, as the widgets hold it.
///
/// Recipients are `String`s, not `Vec<Address>`, because that is what a text box contains. The
/// user is mid-typing for most of this struct's life, and a half-typed address is not an
/// `Address` — forcing it to be one means either rejecting every keystroke or inventing a
/// parse that silently discards what was typed. Parsing happens once, on save, in
/// [`parse_addresses`].
///
/// No `Default`: a composer with no draft behind it is not a state this can be in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composing {
    /// The draft this edits. It already exists in the store before the composer opens.
    pub draft: DraftId,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    /// What went wrong with the last save or send, shown inline.
    pub notice: Option<String>,
    /// Discard has been asked for once and is waiting to be meant.
    ///
    /// The button sits beside Close, and it now deletes the draft rather than merely closing
    /// the pane, so a mis-aimed click would destroy something that no longer exists anywhere
    /// else. One extra click is the whole of the protection, which is what every client that
    /// cannot offer undo does.
    pub confirming_discard: bool,
}

impl Composing {
    /// Open the composer on an existing draft.
    pub fn of(draft: &Draft) -> Self {
        Self {
            draft: draft.id,
            to: join_addresses(&draft.to),
            cc: join_addresses(&draft.cc),
            subject: draft.subject.clone(),
            body: draft.text.clone(),
            notice: None,
            confirming_discard: false,
        }
    }

    /// The edited draft, or what is wrong with it.
    ///
    /// Takes the stored draft rather than building one, so everything the composer does not
    /// show — the identity, the message being replied to, the attachments — survives an edit
    /// instead of being reset to a default the user never chose.
    pub fn apply_to(
        &self,
        base: &Draft,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Draft, String> {
        let to = parse_addresses(&self.to).map_err(|e| format!("To: {e}"))?;
        let cc = parse_addresses(&self.cc).map_err(|e| format!("Cc: {e}"))?;
        Ok(Draft {
            to,
            cc,
            subject: self.subject.clone(),
            text: self.body.clone(),
            updated: now,
            ..base.clone()
        })
    }
}

/// Render addresses back into something a text box can hold.
pub fn join_addresses(list: &[Address]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(name) if !name.is_empty() => format!("{name} <{}>", a.email),
            _ => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a recipient box into addresses.
///
/// Accepts `a@b.test`, `Name <a@b.test>` and `"Name" <a@b.test>`, separated by commas or
/// semicolons. Empty entries are skipped, because a trailing comma is what typing looks like.
///
/// An entry with no `@` is an error rather than a guess. The alternative — dropping it, or
/// appending a default domain — means the user sees their recipient vanish, or sends to
/// someone they did not name. Both are silent; an error is not.
pub fn parse_addresses(input: &str) -> Result<Vec<Address>, String> {
    let mut out: Vec<Address> = Vec::new();
    for entry in split_entries(input) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let address = parse_one(entry)?;
        // First spelling wins, matching `mail_mime::posting`. Two RCPT TO lines for one mailbox
        // are two deliveries.
        if !out
            .iter()
            .any(|kept| kept.email.eq_ignore_ascii_case(&address.email))
        {
            out.push(address);
        }
    }
    Ok(out)
}

/// Split a recipient box on separators that are not inside a display name or an address.
///
/// Not `split([',', ';'])`. `"Lovelace, Ada" <ada@example.test>` is one recipient, and every
/// other mail client writes a display name containing a comma exactly that way — so a naive
/// split turns a pasted recipient into two, one of which is not an address at all. The comma
/// inside quotes is the single most common character this has to *not* split on.
fn split_entries(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut angled) = (0usize, false, false);
    for (i, ch) in input.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            // An unbalanced '<' would otherwise swallow the rest of the box; '>' only closes
            // what a '<' opened, so a stray '>' cannot re-enable splitting that was never off.
            '<' if !quoted => angled = true,
            '>' if !quoted => angled = false,
            ',' | ';' if !quoted && !angled => {
                out.push(&input[start..i]);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&input[start..]);
    out
}

fn parse_one(entry: &str) -> Result<Address, String> {
    let (name, email) = match (entry.rfind('<'), entry.rfind('>')) {
        (Some(open), Some(close)) if close > open => {
            let name = entry[..open].trim().trim_matches('"').trim();
            (
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_owned())
                },
                entry[open + 1..close].trim(),
            )
        }
        _ => (None, entry),
    };
    if email.is_empty() {
        return Err(format!("{entry:?} has no address"));
    }
    // One `@`, with something either side. Not a full RFC 5322 validation: that accepts things
    // no mail server does, and rejecting what a user's own server accepts is worse than letting
    // the server answer. This catches the mistake people actually make, which is a missing `@`.
    let mut halves = email.split('@');
    let (local, domain) = (halves.next().unwrap_or(""), halves.next().unwrap_or(""));
    if local.is_empty() || domain.is_empty() || halves.next().is_some() {
        return Err(format!("{email:?} is not an email address"));
    }
    if email.contains(char::is_whitespace) {
        return Err(format!("{email:?} contains a space"));
    }
    Ok(Address {
        name,
        email: email.to_owned(),
    })
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            places: default_places(),
            selected: 0,
            search: String::new(),
            open: None,
            show_remote_images: false,
            composing: None,
        }
    }
}

impl Shell {
    /// The query the list pane should run.
    ///
    /// A non-empty search box replaces the place rather than narrowing it, which is what every
    /// mail client does and what users expect: searching while in Archive should not hide
    /// results that live in the Inbox.
    pub fn query(&self, limit: u32) -> Query {
        let needle = self.search.trim();
        let filter = if needle.is_empty() {
            match self.places.get(self.selected).map(|place| &place.source) {
                Some(Source::Mail(filter)) => filter.clone(),
                // Reachable only if a caller asks for a query while Drafts is selected.
                // `listing` is the method that knows the difference; this stays total rather
                // than panicking, and `All` is the least surprising thing to show.
                Some(Source::Drafts) | None => Filter::All,
            }
        } else {
            Filter::Text(TextMatch::Contains(needle.to_owned()))
        };
        Query {
            filter,
            sort: Sort {
                property: Property::Date,
                dir: SortDir::Desc,
            },
            page: PageReq { after: None, limit },
        }
    }

    /// What the list pane should show.
    ///
    /// A search box with anything in it always means threads, even while Drafts is selected:
    /// searching is global here, and a user who types into it is looking for a message, not
    /// filtering the drafts they can already see.
    pub fn listing(&self, limit: u32) -> Listing {
        if self.search.trim().is_empty()
            && matches!(
                self.places.get(self.selected).map(|p| &p.source),
                Some(Source::Drafts)
            )
        {
            return Listing::Drafts;
        }
        Listing::Threads(self.query(limit))
    }

    /// Select a place, and drop any open thread that no longer belongs to the new list.
    pub fn select(&mut self, index: usize) {
        if index < self.places.len() {
            self.selected = index;
            self.open = None;
            // Consent is per thread, so changing what is shown revokes it.
            self.show_remote_images = false;
        }
    }

    /// Open a thread.
    pub fn open(&mut self, thread: ThreadId) {
        self.open = Some(thread);
        self.show_remote_images = false;
    }

    /// Open the composer on `draft`.
    pub fn compose(&mut self, draft: &Draft) {
        self.composing = Some(Composing::of(draft));
    }

    /// Drop the composer's widgets without writing them anywhere.
    ///
    /// This **loses** whatever has not been saved, so the only caller that may reach it without
    /// saving first is an explicit Discard. Closing saves and then calls this; an earlier
    /// version closed straight into it and quietly threw away everything typed since the last
    /// Save, behind a comment claiming that could not happen.
    pub fn close_composer(&mut self) {
        self.composing = None;
    }

    /// The sanitizer policy for the thread currently open.
    pub fn policy(&self) -> SanitizePolicy {
        SanitizePolicy {
            remote_images: if self.show_remote_images {
                RemoteImages::Allowed
            } else {
                RemoteImages::Blocked
            },
            version: SanitizePolicy::CURRENT.version,
        }
    }
}

/// What the hover strip offers for a thread.
///
/// Derived from where the thread is, so Archive does not offer "archive" and Trash offers
/// "restore". Returns [`OpKind`] rather than [`Op`] because a button cannot carry a payload
/// that does not exist yet.
pub fn hover_actions(summary: &ThreadSummary) -> Vec<OpKind> {
    let mut out = Vec::new();
    if summary.mailboxes.contains(MailboxRole::Inbox) {
        out.push(OpKind::Archive);
    }
    if summary.mailboxes.contains(MailboxRole::Trash)
        || summary.mailboxes.contains(MailboxRole::Archive)
        || summary.mailboxes.contains(MailboxRole::Spam)
    {
        out.push(OpKind::Restore);
    }
    if !summary.mailboxes.contains(MailboxRole::Trash) {
        out.push(OpKind::Trash);
    }
    out.push(match summary.read {
        ReadState::Unread => OpKind::MarkRead,
        ReadState::Read => OpKind::MarkUnread,
    });
    out.push(match summary.star {
        Star::Unstarred => OpKind::Star,
        Star::Starred => OpKind::Unstar,
    });
    // Always offered. A forward does not depend on where the conversation is or what state it is
    // in — it is the message being passed on, and it was reachable from nowhere until now.
    out.push(OpKind::Forward);
    out
}

/// What a keystroke means.
///
/// The shell had no keyboard at all: not a key handler anywhere in it, so moving between
/// conversations, opening one, archiving, starring, replying and closing a half-written reply
/// were each a mouse click and nothing else. A mail client is a thing people spend hours a day
/// in, and this is the part of "daily driver" that does not depend on anyone's taste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    /// Move to the next conversation and open it.
    Next,
    /// Move to the previous one.
    Previous,
    /// Close the composer if one is open, otherwise close the reader.
    Back,
    /// Archive the open conversation.
    Archive,
    /// Move it to the trash.
    Trash,
    /// Star it, or unstar it if it is already starred.
    ToggleStar,
    /// Mark it read, or unread if it is already read.
    ToggleRead,
    /// Reply to its newest message.
    Reply,
    /// Reply to everyone on it.
    ReplyAll,
    /// Forward it, with no recipients chosen yet.
    Forward,
}

/// The shortcut a key press means, or `None` for a key that is not one.
///
/// `typing` is the whole of the safety here. A letter is a shortcut when the user is reading and
/// a letter when they are writing, and a client that gets that wrong archives a conversation
/// because someone typed "e" into a reply. Only `Escape` survives it — closing what you are
/// typing in is the one thing you must be able to do from inside it.
///
/// Keys are named as the DOM names them, so the caller does not have to invent a second
/// vocabulary for the same events.
pub fn shortcut(key: &str, typing: bool) -> Option<Shortcut> {
    if key == "Escape" {
        return Some(Shortcut::Back);
    }
    if typing {
        return None;
    }
    Some(match key {
        "j" | "ArrowDown" => Shortcut::Next,
        "k" | "ArrowUp" => Shortcut::Previous,
        "e" => Shortcut::Archive,
        "#" | "Delete" => Shortcut::Trash,
        "s" => Shortcut::ToggleStar,
        "u" => Shortcut::ToggleRead,
        "r" => Shortcut::Reply,
        "a" => Shortcut::ReplyAll,
        "f" => Shortcut::Forward,
        _ => return None,
    })
}

/// The operation a shortcut performs on this conversation, if it is one the conversation allows.
///
/// Resolved through [`hover_actions`] rather than by a second table, so the keyboard can reach
/// exactly what the row's own buttons offer and nothing else — no un-archiving something that
/// was never in the inbox, and no starring something that is already starred.
///
/// `None` for [`Shortcut::Reply`] and [`Shortcut::ReplyAll`], which open a composer rather than
/// performing an operation, and for the movement keys.
pub fn op_for_shortcut(shortcut: Shortcut, summary: &ThreadSummary) -> Option<OpKind> {
    let offered = hover_actions(summary);
    let wanted: &[OpKind] = match shortcut {
        Shortcut::Archive => &[OpKind::Archive],
        Shortcut::Trash => &[OpKind::Trash],
        Shortcut::ToggleStar => &[OpKind::Star, OpKind::Unstar],
        Shortcut::ToggleRead => &[OpKind::MarkRead, OpKind::MarkUnread],
        Shortcut::Next | Shortcut::Previous | Shortcut::Back => &[],
        Shortcut::Reply | Shortcut::ReplyAll | Shortcut::Forward => &[],
    };
    wanted.iter().copied().find(|op| offered.contains(op))
}

/// The conversation `Next` or `Previous` moves to.
///
/// Does not wrap. A list that jumps from the bottom back to the top loses the user's place in a
/// way that is hard to notice and easy to act on — the next keystroke archives the wrong thing.
/// Nothing open means the end you are coming from: the first row going down, the last going up.
pub fn step(current: Option<ThreadId>, ids: &[ThreadId], forward: bool) -> Option<ThreadId> {
    if ids.is_empty() {
        return None;
    }
    let Some(here) = current.and_then(|id| ids.iter().position(|it| *it == id)) else {
        // Nothing open, or something that is no longer in the list — archived out from under
        // the selection, most often. Start from the end the movement comes from.
        return Some(if forward { ids[0] } else { ids[ids.len() - 1] });
    };
    let next = if forward {
        here.checked_add(1).filter(|i| *i < ids.len())
    } else {
        here.checked_sub(1)
    };
    Some(ids[next.unwrap_or(here)])
}

/// The `Op` a hover button performs, where it needs no payload.
///
/// `None` for the ones that open a composer instead — a reply is a draft, not an operation.
pub fn op_for(kind: OpKind) -> Option<Op> {
    match kind {
        OpKind::Archive => Some(Op::Archive),
        OpKind::Trash => Some(Op::Trash),
        OpKind::Restore => Some(Op::Restore),
        OpKind::Spam => Some(Op::Spam),
        OpKind::MarkRead => Some(Op::SetRead(ReadState::Read)),
        OpKind::MarkUnread => Some(Op::SetRead(ReadState::Unread)),
        OpKind::Star => Some(Op::SetStar(Star::Starred)),
        OpKind::Unstar => Some(Op::SetStar(Star::Unstarred)),
        // These need a label picked, a draft created, or a date chosen.
        OpKind::AddLabel
        | OpKind::RemoveLabel
        | OpKind::Snooze
        | OpKind::Pin
        | OpKind::Reply
        | OpKind::ReplyAll
        | OpKind::Forward => None,
    }
}

/// The message a reply to this thread should answer.
///
/// The newest, which is what "reply" means to everyone except the person who wrote the
/// threading code. Replying to the thread's *root* would quote a conversation's opening line
/// back at someone who has since sent four more, and would set `In-Reply-To` to a message the
/// recipient's client threads above everything they last read.
///
/// Ties break on id so a thread with two messages at the same timestamp — which a bulk import
/// produces easily — picks the same one every time rather than alternating between renders.
pub fn reply_target(messages: &[Message]) -> Option<&Message> {
    messages.iter().max_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
    })
}

/// Where a sync pass has got to, as the window shows it.
///
/// A signal the UI reads, not a channel it polls. The pass itself runs on a blocking thread
/// because it opens sockets and a SQLite connection; what crosses back is this, and only this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Idle,
    Running,
    /// Finished, with what the pass reported.
    Done(String),
    Failed(String),
}

impl SyncState {
    /// Whether a new pass may start.
    ///
    /// Two concurrent passes on one account would fetch the same messages twice and race each
    /// other's writes for the same rows. The button is disabled rather than queueing, because a
    /// second sync a user asked for while the first was running is the same request, not
    /// another one.
    pub fn may_start(&self) -> bool {
        !matches!(self, SyncState::Running)
    }

    /// What the status line should say, or `None` when there is nothing to report.
    pub fn message(&self) -> Option<&str> {
        match self {
            SyncState::Idle => None,
            SyncState::Running => Some("Syncing…"),
            SyncState::Done(text) | SyncState::Failed(text) => Some(text.trim_end()),
        }
    }

    /// Whether the message describes a failure, so the window can style it as one.
    pub fn is_failure(&self) -> bool {
        matches!(self, SyncState::Failed(_))
    }
}

/// Turn a finished pass into the state to display.
///
/// `Ok("")` becomes `Done("Up to date.")` rather than an empty status line: a sync that
/// reported nothing still happened, and a blank line reads as "the button did nothing".
pub fn synced(result: Result<String, String>) -> SyncState {
    match result {
        Ok(text) if text.trim().is_empty() => SyncState::Done("Up to date.".to_owned()),
        Ok(text) => SyncState::Done(text),
        Err(why) => SyncState::Failed(why),
    }
}

/// What the list pane should render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    Threads(Query),
    /// The drafts table, which no `Query` can express.
    Drafts,
}

/// What the reader should display for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// Headers only so far. Normal mid-sync, and not an empty message.
    NotFetched,
    /// Plain text, to render as text.
    Text(String),
    /// Sanitized markup, for a sandboxed frame.
    Html {
        html: String,
        /// Whether the sanitizer removed a remote fetch. The reader offers to load them only
        /// when there is something to load — the offer used to appear on every message, plain
        /// text included, which is how a security control becomes furniture.
        blocked_remote: bool,
    },
}

/// Decide what to show for a message body.
///
/// HTML is sanitized here rather than at ingest, so a policy change takes effect immediately
/// and an `ammonia` upgrade does not leave old messages sanitized under old rules.
pub fn reading(body: &Body, html: Option<&str>, policy: SanitizePolicy) -> Reading {
    match body {
        Body::Absent => Reading::NotFetched,
        Body::Present { text, .. } => match (html, text) {
            (Some(raw), _) => {
                let safe = mail_mime::sanitize(raw, policy);
                Reading::Html {
                    html: safe.as_str().to_owned(),
                    blocked_remote: safe.blocked_remote() > 0,
                }
            }
            (None, Some(text)) => Reading::Text(text.clone()),
            (None, None) => Reading::Text(String::new()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn summary(tweak: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut s = ThreadSummary {
            id: ThreadId::generate(),
            account: AccountId::generate(),
            subject: "s".into(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "a@b.test".into(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            message_count: 1,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
        };
        tweak(&mut s);
        s
    }

    /// Whether a thread matches, with no corpus — none of these filters reads one.
    fn fits(filter: &Filter, summary: &ThreadSummary, now: DateTime<Utc>) -> bool {
        filter.fit(&mail_domain::MatchCtx {
            summary,
            corpus: None,
            now,
        })
    }

    #[test]
    fn inbox_is_just_a_filter() {
        // The plan claims a place is a saved filter rather than anything special. Assert it —
        // including the clause that makes snoozing mean something, which is still a filter and
        // not a special case in the list.
        let shell = Shell::default();
        assert_eq!(
            shell.query(20).filter,
            Filter::And(vec![
                Filter::InMailbox(MailboxRole::Inbox),
                Filter::Not(Box::new(pending_snooze())),
            ])
        );
    }

    #[test]
    fn a_snoozed_conversation_is_away_from_the_inbox_until_its_hour() {
        // Against `Filter::fit`, which is the same predicate the SQL side agrees with by
        // proptest. A button that leaves the conversation in the list does nothing.
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap();
        let inbox = Shell::default().query(20).filter;

        let awake = summary(|_| {});
        let away =
            summary(|s| s.snooze = Snooze::Until(now + chrono::TimeDelta::try_hours(3).unwrap()));
        let due =
            summary(|s| s.snooze = Snooze::Until(now - chrono::TimeDelta::try_hours(1).unwrap()));

        assert!(
            fits(&inbox, &awake, now),
            "an ordinary thread is in the inbox"
        );
        assert!(
            !fits(&inbox, &away, now),
            "a snoozed thread is still listed"
        );
        assert!(
            fits(&inbox, &due, now),
            "a snooze that has passed did not bring it back"
        );
    }

    #[test]
    fn the_snoozed_place_holds_only_what_is_still_away() {
        // A due thread is back in the inbox; listing it here as well would make "snoozed" mean
        // two different things in two places.
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap();
        let snoozed = pending_snooze();
        let away =
            summary(|s| s.snooze = Snooze::Until(now + chrono::TimeDelta::try_hours(3).unwrap()));
        let due =
            summary(|s| s.snooze = Snooze::Until(now - chrono::TimeDelta::try_hours(1).unwrap()));

        assert!(fits(&snoozed, &away, now));
        assert!(!fits(&snoozed, &due, now));
        assert!(!fits(&snoozed, &summary(|_| {}), now));
    }

    #[test]
    fn the_sidebar_offers_somewhere_to_find_a_snoozed_conversation() {
        // Otherwise snoozing is a way to lose mail.
        let places = default_places();
        let snoozed = places
            .iter()
            .find(|p| p.name == "Snoozed")
            .expect("no Snoozed place");
        assert_eq!(snoozed.source, Source::Mail(pending_snooze()));
    }

    #[test]
    fn searching_replaces_the_place_rather_than_narrowing_it() {
        // Searching while in Archive must not hide a result that lives in the Inbox.
        let mut shell = Shell::default();
        shell.select(1);
        shell.search = "  lunch  ".to_owned();
        assert_eq!(
            shell.query(20).filter,
            Filter::Text(TextMatch::Contains("lunch".to_owned())),
            "a search is global, and the box is trimmed"
        );
    }

    #[test]
    fn an_empty_search_box_falls_back_to_the_place() {
        let mut shell = Shell::default();
        shell.select(2);
        shell.search = "   ".to_owned();
        assert_eq!(shell.query(20).filter, Filter::InMailbox(MailboxRole::Sent));
    }

    #[test]
    fn consent_to_remote_images_does_not_survive_changing_what_is_shown() {
        // A remote image is a read receipt. Agreeing to load one sender's is not agreeing to
        // the next message's, so consent is revoked by opening anything else.
        let mut shell = Shell {
            show_remote_images: true,
            ..Shell::default()
        };
        shell.open(ThreadId::generate());
        assert!(!shell.show_remote_images);

        shell.show_remote_images = true;
        shell.select(1);
        assert!(!shell.show_remote_images);
        assert_eq!(shell.policy().remote_images, RemoteImages::Blocked);
    }

    #[test]
    fn hover_actions_fit_where_the_thread_is() {
        let inbox = hover_actions(&summary(|_| {}));
        assert!(inbox.contains(&OpKind::Archive), "{inbox:?}");
        assert!(!inbox.contains(&OpKind::Restore), "nothing to restore from");

        let archived = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Archive)
        }));
        assert!(archived.contains(&OpKind::Restore), "{archived:?}");
        assert!(
            !archived.contains(&OpKind::Archive),
            "offering to archive what is archived is noise"
        );

        let trashed = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Trash)
        }));
        assert!(!trashed.contains(&OpKind::Trash), "{trashed:?}");
    }

    #[test]
    fn the_read_and_star_buttons_show_the_opposite_of_the_current_state() {
        let unread = hover_actions(&summary(|_| {}));
        assert!(unread.contains(&OpKind::MarkRead));
        let read = hover_actions(&summary(|s| s.read = ReadState::Read));
        assert!(read.contains(&OpKind::MarkUnread));
    }

    #[test]
    fn actions_needing_a_payload_have_no_bare_op() {
        // A reply is a draft, not an operation, so a hover button cannot perform one.
        for kind in [
            OpKind::Reply,
            OpKind::Forward,
            OpKind::AddLabel,
            OpKind::Snooze,
        ] {
            assert!(op_for(kind).is_none(), "{kind:?} should open something");
        }
        assert_eq!(op_for(OpKind::Archive), Some(Op::Archive));
    }

    #[test]
    fn an_unfetched_body_is_not_an_empty_message() {
        assert_eq!(
            reading(&Body::Absent, None, SanitizePolicy::CURRENT),
            Reading::NotFetched
        );
    }

    #[test]
    fn html_is_sanitized_at_render_and_the_policy_is_honoured() {
        let body = Body::Present {
            text: Some("fallback".into()),
            raw: BlobId::generate(),
        };
        let hostile = r#"<p>hi</p><script>alert(1)</script><img src="https://tracker.test/p.gif">"#;

        let blocked = reading(&body, Some(hostile), SanitizePolicy::CURRENT);
        let Reading::Html { html: rendered, .. } = blocked else {
            panic!("html should render as html");
        };
        assert!(rendered.contains("<p>hi</p>"), "{rendered}");
        assert!(!rendered.contains("alert"), "script survived: {rendered}");
        assert!(
            !rendered.contains("tracker.test"),
            "a remote image survived blocking: {rendered}"
        );

        let allowed = reading(
            &body,
            Some(hostile),
            SanitizePolicy {
                remote_images: RemoteImages::Allowed,
                version: SanitizePolicy::CURRENT.version,
            },
        );
        let Reading::Html { html: rendered, .. } = allowed else {
            panic!("html should render as html");
        };
        assert!(rendered.contains("tracker.test"), "opting in must work");
        assert!(!rendered.contains("alert"), "images are not scripts");
    }

    #[test]
    fn plain_text_is_used_when_there_is_no_html() {
        let body = Body::Present {
            text: Some("just text".into()),
            raw: BlobId::generate(),
        };
        assert_eq!(
            reading(&body, None, SanitizePolicy::CURRENT),
            Reading::Text("just text".into())
        );
    }
}

#[cfg(test)]
mod composer_tests {
    use super::*;

    fn addr(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    #[test]
    fn a_recipient_box_accepts_the_shapes_people_type() {
        assert_eq!(
            parse_addresses("ada@example.test").unwrap(),
            vec![addr(None, "ada@example.test")]
        );
        assert_eq!(
            parse_addresses("Ada Lovelace <ada@example.test>").unwrap(),
            vec![addr(Some("Ada Lovelace"), "ada@example.test")]
        );
        // Quoted display names are what other clients paste in.
        assert_eq!(
            parse_addresses("\"Lovelace, Ada\" <ada@example.test>").unwrap(),
            vec![addr(Some("Lovelace, Ada"), "ada@example.test")]
        );
    }

    #[test]
    fn a_comma_inside_a_display_name_does_not_split_the_recipient() {
        // Every other client writes "Last, First" this way, so this arrives by paste constantly.
        // Splitting here turns one recipient into two, one of which is not an address at all.
        let parsed = parse_addresses("\"Lovelace, Ada\" <ada@x.test>, bob@x.test").unwrap();
        assert_eq!(parsed.len(), 2, "{parsed:?}");
        assert_eq!(parsed[0].name.as_deref(), Some("Lovelace, Ada"));
        assert_eq!(parsed[0].email, "ada@x.test");
        assert_eq!(parsed[1].email, "bob@x.test");
    }

    #[test]
    fn a_separator_inside_the_address_itself_does_not_split_either() {
        let parsed = parse_addresses("<a;b@x.test>").unwrap();
        assert_eq!(parsed.len(), 1, "{parsed:?}");
    }

    #[test]
    fn commas_and_semicolons_both_separate() {
        // Outlook produces semicolons. A user pasting from it should not have to know.
        let by_comma = parse_addresses("a@x.test, b@x.test").unwrap();
        let by_semi = parse_addresses("a@x.test; b@x.test").unwrap();
        assert_eq!(by_comma, by_semi);
        assert_eq!(by_comma.len(), 2);
    }

    #[test]
    fn a_trailing_comma_is_what_typing_looks_like() {
        // The box is parsed on every keystroke to show errors. If "a@x.test," were an error,
        // the composer would shout at the user in the middle of typing the second recipient.
        assert_eq!(parse_addresses("a@x.test,").unwrap().len(), 1);
        assert_eq!(parse_addresses("  ,, a@x.test , ,").unwrap().len(), 1);
        assert!(parse_addresses("").unwrap().is_empty());
        assert!(parse_addresses("   ").unwrap().is_empty());
    }

    #[test]
    fn something_that_is_not_an_address_is_an_error_not_a_guess() {
        // Dropping it makes the recipient vanish; appending a default domain sends to someone
        // the user did not name. Both are silent, which is what makes them worse than an error.
        for bad in ["ada", "ada@", "@example.test", "a@b@c", "ada example.test"] {
            assert!(
                parse_addresses(bad).is_err(),
                "{bad:?} was accepted as an address"
            );
        }
    }

    #[test]
    fn a_bad_entry_fails_the_whole_box_rather_than_half_of_it() {
        // Sending to "everyone I could parse" is the failure mode this prevents.
        let err = parse_addresses("good@x.test, nonsense, other@x.test").unwrap_err();
        assert!(err.contains("nonsense"), "{err}");
    }

    #[test]
    fn one_mailbox_twice_is_one_recipient() {
        let parsed = parse_addresses("Ada <ada@x.test>, ADA@X.TEST").unwrap();
        assert_eq!(parsed.len(), 1, "{parsed:?}");
        // The first spelling wins, so the display name the user typed survives.
        assert_eq!(parsed[0].name.as_deref(), Some("Ada"));
    }

    #[test]
    fn what_the_box_shows_parses_back_to_what_it_held() {
        let original = vec![
            addr(Some("Ada Lovelace"), "ada@example.test"),
            addr(None, "bob@example.test"),
        ];
        assert_eq!(
            parse_addresses(&join_addresses(&original)).unwrap(),
            original
        );
    }

    fn draft_of(to: Vec<Address>) -> Draft {
        Draft {
            id: DraftId::generate(),
            account: AccountId::generate(),
            identity: IdentityId::generate(),
            to,
            cc: Vec::new(),
            bcc: vec![addr(None, "blind@example.test")],
            subject: "Re: lunch".to_owned(),
            in_reply_to: Some(MessageId::generate()),
            forward_of: None,
            text: "body".to_owned(),
            html: None,
            attachments: Vec::new(),
            state: SendState::Editing,
            updated: chrono::Utc::now(),
        }
    }

    #[test]
    fn editing_changes_only_what_the_composer_shows() {
        // The composer has no Bcc box, no identity picker and no attachment list. If it built a
        // Draft from scratch it would silently drop all three — and the blind recipient would
        // stop receiving the message because the user fixed a typo in the subject.
        let base = draft_of(vec![addr(None, "ada@example.test")]);
        let mut editing = Composing::of(&base);
        editing.subject = "Re: lunch, moved".to_owned();
        let now = chrono::Utc::now();

        let edited = editing.apply_to(&base, now).unwrap();
        assert_eq!(edited.subject, "Re: lunch, moved");
        assert_eq!(edited.bcc, base.bcc, "the blind recipient was dropped");
        assert_eq!(edited.identity, base.identity);
        assert_eq!(edited.in_reply_to, base.in_reply_to, "threading was lost");
        assert_eq!(edited.id, base.id, "editing must not mint a second draft");
        assert_eq!(edited.updated, now);
    }

    #[test]
    fn a_bad_recipient_names_which_box_it_is_in() {
        let base = draft_of(Vec::new());
        let mut editing = Composing::of(&base);
        editing.to = "fine@example.test".to_owned();
        editing.cc = "not-an-address".to_owned();

        let err = editing
            .apply_to(&base, chrono::Utc::now())
            .expect_err("the Cc box is wrong");
        assert!(err.starts_with("Cc:"), "{err}");
    }

    #[test]
    fn opening_the_composer_shows_what_the_draft_holds() {
        let base = draft_of(vec![addr(Some("Ada"), "ada@example.test")]);
        let editing = Composing::of(&base);
        assert_eq!(editing.to, "Ada <ada@example.test>");
        assert_eq!(editing.subject, "Re: lunch");
        assert_eq!(editing.body, "body");
        assert_eq!(editing.draft, base.id);
        assert_eq!(editing.notice, None);
    }

    #[test]
    fn a_half_written_reply_survives_reading_another_thread() {
        // Composing sits beside reading rather than replacing it. A mode enum here would throw
        // the draft's widgets away the moment the user clicked another conversation.
        let base = draft_of(vec![addr(None, "ada@example.test")]);
        let mut shell = Shell::default();
        shell.compose(&base);
        shell.composing.as_mut().unwrap().body = "half a sentence".to_owned();

        shell.open(ThreadId::generate());
        assert_eq!(
            shell.composing.as_ref().map(|c| c.body.as_str()),
            Some("half a sentence"),
            "opening a thread discarded the composer"
        );

        shell.close_composer();
        assert!(shell.composing.is_none());
    }
}

#[cfg(test)]
mod reply_target_tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn message(n: i64) -> Message {
        Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: AccountId::generate(),
            key: MessageKey::Rfc(format!("m{n}@example.test")),
            date: Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap(),
            from: Address {
                name: None,
                email: format!("s{n}@example.test"),
            },
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: format!("subject {n}"),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: Some(format!("m{n}@example.test")),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Absent,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn a_reply_answers_the_newest_message_not_the_first() {
        // Replying to the root quotes a conversation's opening line back at someone who has
        // since sent four more, and threads the reply above everything they last read.
        let messages = vec![message(0), message(300), message(100)];
        let target = reply_target(&messages).expect("a thread has messages");
        assert_eq!(target.subject, "subject 300");
    }

    #[test]
    fn the_choice_is_stable_when_two_messages_share_a_timestamp() {
        // A bulk import produces these easily. An unstable pick would reply to a different
        // message on each render, which the user would see as the quote changing under them.
        let messages = vec![message(5), message(5), message(5)];
        let first = reply_target(&messages).unwrap().id;
        for _ in 0..8 {
            assert_eq!(reply_target(&messages).unwrap().id, first);
        }
    }

    #[test]
    fn an_empty_thread_has_nothing_to_reply_to() {
        // Reachable: the messages are loaded one by one and any of them can fail to read.
        assert!(reply_target(&[]).is_none());
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    fn drafts_index(shell: &Shell) -> usize {
        shell
            .places
            .iter()
            .position(|p| p.source == Source::Drafts)
            .expect("there is a Drafts place")
    }

    #[test]
    fn the_drafts_place_lists_drafts_and_not_an_empty_mailbox() {
        // The bug this replaces: Drafts was `Filter::InMailbox(MailboxRole::Drafts)`, and a
        // draft has no thread and no mailbox, so the pane showed nothing for ever and said
        // nothing about why.
        let mut shell = Shell::default();
        shell.select(drafts_index(&shell));
        assert_eq!(shell.listing(50), Listing::Drafts);
    }

    #[test]
    fn every_other_place_still_lists_threads() {
        let shell = Shell::default();
        for (index, place) in shell.places.iter().enumerate() {
            if place.source == Source::Drafts {
                continue;
            }
            let mut shell = Shell::default();
            shell.select(index);
            assert!(
                matches!(shell.listing(50), Listing::Threads(_)),
                "{} stopped listing threads",
                place.name
            );
        }
    }

    #[test]
    fn searching_while_in_drafts_searches_mail() {
        // Search is global. Someone typing in the box is looking for a message, not filtering
        // the handful of drafts already on screen.
        let mut shell = Shell::default();
        shell.select(drafts_index(&shell));
        shell.search = "invoice".to_owned();

        match shell.listing(50) {
            Listing::Threads(query) => assert_eq!(
                query.filter,
                Filter::Text(TextMatch::Contains("invoice".to_owned()))
            ),
            Listing::Drafts => panic!("a search in Drafts must still search mail"),
        }
    }

    #[test]
    fn a_blank_search_box_goes_back_to_the_drafts_list() {
        // Whitespace only is a blank box, not a search for a space.
        let mut shell = Shell::default();
        shell.select(drafts_index(&shell));
        shell.search = "   ".to_owned();
        assert_eq!(shell.listing(50), Listing::Drafts);
    }

    #[test]
    fn the_page_limit_reaches_the_query() {
        // "Show more" works by asking for a bigger page, so a limit that did not travel would
        // make the button do nothing at all.
        let shell = Shell::default();
        match shell.listing(250) {
            Listing::Threads(query) => assert_eq!(query.page.limit, 250),
            Listing::Drafts => panic!("the Inbox is not the drafts list"),
        }
    }
}

#[cfg(test)]
mod sync_state_tests {
    use super::*;

    #[test]
    fn a_second_sync_cannot_start_while_one_is_running() {
        // Two passes on one account fetch the same messages twice and race each other's writes.
        assert!(SyncState::Idle.may_start());
        assert!(!SyncState::Running.may_start());
        assert!(SyncState::Done("done".to_owned()).may_start());
        assert!(SyncState::Failed("nope".to_owned()).may_start());
    }

    #[test]
    fn a_pass_that_reported_nothing_still_says_something() {
        // An empty status line reads as "the button did nothing".
        assert_eq!(
            synced(Ok(String::new())).message(),
            Some("Up to date."),
            "a silent success must not look like a no-op"
        );
        assert_eq!(
            synced(Ok("   \n".to_owned())).message(),
            Some("Up to date.")
        );
    }

    #[test]
    fn what_the_pass_reported_is_what_is_shown() {
        let state = synced(Ok("me@x.test: 3 headers, 3 bodies\n".to_owned()));
        assert_eq!(state.message(), Some("me@x.test: 3 headers, 3 bodies"));
        assert!(!state.is_failure());
    }

    #[test]
    fn a_failure_is_shown_as_one_rather_than_swallowed() {
        // The failure most likely here is "no credential", which is fixable — but only by
        // someone who is told about it.
        let state = synced(Err("no credential stored".to_owned()));
        assert!(state.is_failure());
        assert_eq!(state.message(), Some("no credential stored"));
    }

    #[test]
    fn idle_says_nothing_at_all() {
        assert_eq!(SyncState::Idle.message(), None);
    }
}

/// Where an instant appears, and therefore how much of it is written.
///
/// An enum rather than a format string at each call site, because the call sites disagreed
/// about the pattern and agreed about the bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// A row in a list: `09-22 09:02`. The year is omitted; the column is narrow and the
    /// question a list answers is "when today".
    Row,
    /// A message in the reader: `2026-09-22 09:02`. Opened deliberately, so it says everything.
    Full,
    /// The attribution line above quoted text: `Tue, 22 Sep 2026 at 09:02`.
    ///
    /// The one stamp that leaves this machine. It is written into the body of a reply, so a
    /// wrong one is wrong in someone else's mailbox, permanently, and no later fix reaches it.
    Quote,
}

impl Stamp {
    fn pattern(self) -> &'static str {
        match self {
            Stamp::Row => "%m-%d %H:%M",
            Stamp::Full => "%Y-%m-%d %H:%M",
            Stamp::Quote => "%a, %d %b %Y at %H:%M",
        }
    }
}

/// Why the list pane has nothing in it, which decides what it should say.
///
/// The pane said "Nothing here." in every case, including the one every new user starts in: no
/// account configured at all. A mail client that has never been told whose mail to fetch looks
/// exactly like a mailbox that happens to be empty, and the shell has no way to add an account
/// — that is a terminal command — so "nothing here" was the end of the road rather than a state
/// with a way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nothing {
    /// No account has been added yet.
    NoAccount,
    /// A search that matched nothing. Carries the words, because what was searched for is the
    /// thing most likely to be mistyped.
    NoMatch(String),
    /// A folder with no mail in it, which is ordinary.
    EmptyFolder,
}

/// Why the list is empty.
pub fn nothing_to_show(accounts: usize, search: &str) -> Nothing {
    let needle = search.trim();
    if accounts == 0 {
        // Checked first: with no account there is nothing to search, and "nothing matches" would
        // send the user looking for a typo instead of for the setup step they have not done.
        Nothing::NoAccount
    } else if !needle.is_empty() {
        Nothing::NoMatch(needle.to_owned())
    } else {
        Nothing::EmptyFolder
    }
}

impl Nothing {
    /// What to say.
    pub fn message(&self) -> String {
        match self {
            Nothing::NoAccount => "No account yet. Add one from a terminal:".to_owned(),
            Nothing::NoMatch(needle) => format!("Nothing matches {needle:?}."),
            Nothing::EmptyFolder => "Nothing here.".to_owned(),
        }
    }

    /// The command that gets the user out of this state, when there is one.
    ///
    /// Separate from the message so the shell can set it in a monospace face rather than in the
    /// italic the rest of the pane uses. A command shown in italic prose is a command someone
    /// retypes wrongly.
    pub fn command(&self) -> Option<&'static str> {
        match self {
            Nothing::NoAccount => Some("mailo account add <address>"),
            Nothing::NoMatch(_) | Nothing::EmptyFolder => None,
        }
    }
}

/// What a click on Discard means, given what the composer is currently showing.
///
/// A value rather than a branch inside the button, for the same reason `op_for` and
/// `hover_actions` are: the decision is the part that can be wrong, and a decision that only
/// exists inside a closure attached to a DOM node cannot be tested without a DOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discarding {
    /// Ask first. Discard deletes the draft now, and the button sits beside Close.
    Confirm,
    /// Asked and meant.
    Delete(DraftId),
}

/// Decide what a click on Discard should do.
///
/// `None` when there is no composer open, which the button cannot reach but the caller should
/// not have to assume.
pub fn discard_click(composing: Option<&Composing>) -> Option<Discarding> {
    let composing = composing?;
    Some(if composing.confirming_discard {
        Discarding::Delete(composing.draft)
    } else {
        Discarding::Confirm
    })
}

/// How a list row writes an instant: precisely enough to be useful, briefly enough to fit.
///
/// The time for today, a weekday for the last week, a day and month within the year, and a full
/// date beyond it. This is what every mail client does and for the same reason — the shell wrote
/// `Sep 22` on a message that arrived an hour ago, which is both the least useful answer
/// available and the same answer it gives for a message from three weeks ago.
///
/// `now` is a parameter for the usual reason: a function that reads the clock decides its own
/// test's answer.
pub fn listed<Tz: TimeZone>(instant: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let then = instant.with_timezone(zone);
    let today = now.with_timezone(zone);
    // Calendar days, not elapsed hours: 23:50 yesterday is not "today" because it is within
    // twenty-four hours, and 00:10 this morning is.
    let days = today
        .date_naive()
        .signed_duration_since(then.date_naive())
        .num_days();
    if days == 0 {
        then.format("%H:%M").to_string()
    } else if (1..7).contains(&days) {
        then.format("%a").to_string()
    } else if then.format("%Y").to_string() == today.format("%Y").to_string() {
        then.format("%b %d").to_string()
    } else {
        // A year old. The year is the only part that still says anything.
        then.format("%Y-%m-%d").to_string()
    }
}

/// Write an instant the way the person reading it keeps time.
///
/// Every instant is stored in UTC, which is the only sane way to keep one, and every instant a
/// person reads is in their own zone. Nothing converted between the two: the list, the reader
/// and the drafts pane each formatted the `DateTime<Utc>` directly. On this machine —
/// `Asia/Taipei`, `+0800` — mail that arrived at 09:02 was shown as 01:02, and every message
/// that arrived after 16:00 was filed under the previous day. At the moment this was found the
/// clock read 07:25 on the 22nd and the whole application was showing the 21st.
///
/// The zone is a parameter and not `Local` read from inside, because a function that reads the
/// machine's zone can only be tested against whatever that machine is set to — which on a
/// machine set to UTC is the bug passing.
pub fn stamp<Tz: TimeZone>(instant: DateTime<Utc>, zone: &Tz, stamp: Stamp) -> String
where
    Tz::Offset: std::fmt::Display,
{
    instant
        .with_timezone(zone)
        .format(stamp.pattern())
        .to_string()
}

#[cfg(test)]
mod badge_tests {
    use super::*;

    #[test]
    fn a_mail_place_counts_its_unread_threads() {
        let inbox = Source::Mail(Filter::InMailbox(MailboxRole::Inbox));
        match badge_filter(&inbox) {
            Some(Filter::And(clauses)) => {
                assert!(clauses.contains(&Filter::InMailbox(MailboxRole::Inbox)));
                assert!(
                    clauses.contains(&Filter::Read(ReadState::Unread)),
                    "the badge counted every thread, not the unread ones: {clauses:?}"
                );
            }
            other => panic!("expected a conjunction, got {other:?}"),
        }
    }

    #[test]
    fn drafts_has_no_unread_badge() {
        // "3 unread drafts" is not a thing: a draft did not arrive and nobody failed to read it.
        assert_eq!(badge_filter(&Source::Drafts), None);
    }

    #[test]
    fn every_mail_place_gets_one_including_archive() {
        // A filter rule can file an unread message straight into Archive. A badgeless Archive is
        // then a message the user never finds out about.
        for place in default_places() {
            match &place.source {
                Source::Mail(_) => assert!(
                    badge_filter(&place.source).is_some(),
                    "{} has no badge",
                    place.name
                ),
                Source::Drafts => assert!(badge_filter(&place.source).is_none()),
            }
        }
    }
}

/// The keyboard, which the shell did not have.
#[cfg(test)]
mod keyboard {
    use super::*;

    fn summary(read: ReadState, star: Star, mailbox: MailboxRole) -> ThreadSummary {
        ThreadSummary {
            id: ThreadId::generate(),
            account: AccountId::generate(),
            subject: "lunch".to_owned(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "ada@example.test".to_owned(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: Utc.with_ymd_and_hms(2026, 9, 22, 0, 0, 0).unwrap(),
            message_count: 1,
            read,
            star,
            mailboxes: MailboxSet::only(mailbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
        }
    }

    #[test]
    fn a_letter_is_a_shortcut_while_reading_and_a_letter_while_writing() {
        // The bug this exists to prevent: typing "e" into a reply archiving the conversation
        // behind it.
        assert_eq!(shortcut("e", false), Some(Shortcut::Archive));
        assert_eq!(shortcut("e", true), None);
        for key in ["j", "k", "s", "u", "r", "a", "#", "ArrowDown", "ArrowUp"] {
            assert!(shortcut(key, false).is_some(), "{key} does nothing");
            assert_eq!(shortcut(key, true), None, "{key} fired while typing");
        }
    }

    #[test]
    fn escape_works_from_inside_the_thing_it_closes() {
        // The one exception, and it has to be: closing what you are typing in is not something
        // you can be asked to reach for the mouse to do.
        assert_eq!(shortcut("Escape", true), Some(Shortcut::Back));
        assert_eq!(shortcut("Escape", false), Some(Shortcut::Back));
    }

    #[test]
    fn a_key_that_is_not_a_shortcut_is_left_alone() {
        for key in ["z", "F5", "Tab", "Shift", " ", "1"] {
            assert_eq!(shortcut(key, false), None, "{key} was swallowed");
        }
    }

    #[test]
    fn star_and_read_resolve_against_what_the_thread_already_is() {
        // Toggles, and resolved through `hover_actions` so the keyboard and the row's buttons
        // cannot disagree about what is possible.
        let unstarred = summary(ReadState::Unread, Star::Unstarred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleStar, &unstarred),
            Some(OpKind::Star)
        );
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleRead, &unstarred),
            Some(OpKind::MarkRead)
        );
        let starred = summary(ReadState::Read, Star::Starred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleStar, &starred),
            Some(OpKind::Unstar)
        );
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleRead, &starred),
            Some(OpKind::MarkUnread)
        );
    }

    #[test]
    fn forward_is_offered_on_every_conversation() {
        // It does not depend on where the conversation is or what state it is in: a forward
        // carries the message. It was reachable from nowhere before this.
        for mailbox in [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Trash,
            MailboxRole::Sent,
        ] {
            let summary = summary(ReadState::Read, Star::Unstarred, mailbox);
            assert!(
                hover_actions(&summary).contains(&OpKind::Forward),
                "no Forward on a conversation in {mailbox:?}"
            );
        }
    }

    #[test]
    fn f_forwards_and_is_not_an_operation() {
        assert_eq!(shortcut("f", false), Some(Shortcut::Forward));
        assert_eq!(shortcut("f", true), None, "fired while typing");
        let inbox = summary(ReadState::Read, Star::Unstarred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::Forward, &inbox),
            None,
            "a forward opens a composer rather than performing an operation"
        );
    }

    #[test]
    fn a_shortcut_cannot_reach_what_the_row_would_not_offer() {
        // Archiving something that is not in the inbox. The buttons do not offer it, so neither
        // does the key — one table, not two.
        let archived = summary(ReadState::Read, Star::Unstarred, MailboxRole::Archive);
        assert_eq!(op_for_shortcut(Shortcut::Archive, &archived), None);
        let inbox = summary(ReadState::Read, Star::Unstarred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::Archive, &inbox),
            Some(OpKind::Archive)
        );
        // And the ones that are not operations at all.
        for shortcut in [Shortcut::Next, Shortcut::Back, Shortcut::Reply] {
            assert_eq!(op_for_shortcut(shortcut, &inbox), None);
        }
    }

    #[test]
    fn moving_stops_at_the_ends_rather_than_wrapping() {
        // A list that jumps from the bottom back to the top loses the user's place in a way that
        // is hard to notice and easy to act on: the next keystroke archives the wrong thing.
        let ids: Vec<ThreadId> = (0..3).map(|_| ThreadId::generate()).collect();
        assert_eq!(step(Some(ids[0]), &ids, true), Some(ids[1]));
        assert_eq!(
            step(Some(ids[2]), &ids, true),
            Some(ids[2]),
            "wrapped forward"
        );
        assert_eq!(step(Some(ids[1]), &ids, false), Some(ids[0]));
        assert_eq!(
            step(Some(ids[0]), &ids, false),
            Some(ids[0]),
            "wrapped back"
        );
    }

    #[test]
    fn moving_with_nothing_open_starts_from_the_end_it_comes_from() {
        let ids: Vec<ThreadId> = (0..3).map(|_| ThreadId::generate()).collect();
        assert_eq!(step(None, &ids, true), Some(ids[0]));
        assert_eq!(step(None, &ids, false), Some(ids[2]));
        assert_eq!(
            step(None, &[], true),
            None,
            "an empty list has nowhere to go"
        );
    }

    #[test]
    fn a_selection_that_left_the_list_does_not_strand_the_keyboard() {
        // Archiving the open conversation removes it from an inbox listing while it is still
        // `Shell::open`. The next keystroke has to go somewhere rather than nowhere.
        let ids: Vec<ThreadId> = (0..2).map(|_| ThreadId::generate()).collect();
        let gone = ThreadId::generate();
        assert_eq!(step(Some(gone), &ids, true), Some(ids[0]));
        assert_eq!(step(Some(gone), &ids, false), Some(ids[1]));
    }
}

/// When a conversation should come back.
///
/// The vocabulary is small on purpose. "Snooze until a quarter past four on the third Tuesday"
/// is a calendar; what a mail client needs is the four or five answers people actually give,
/// plus an exact date for the rest.
///
/// Resolved in the reader's zone and against a `now` the caller supplies — a function that reads
/// the clock decides its own test's answer, and one that assumes UTC sends "tomorrow morning" to
/// the middle of tonight for anyone east of Greenwich.
///
/// | phrase | means |
/// | --- | --- |
/// | `later` | three hours from now |
/// | `tonight` | 19:00 today, or tomorrow if that has passed |
/// | `tomorrow` | 09:00 tomorrow |
/// | `weekend` | 09:00 on the coming Saturday |
/// | `monday` … `sunday` | 09:00 on the next such day |
/// | `+90m`, `+2h`, `+3d` | that much from now |
/// | `2026-09-25` | 09:00 that day |
/// | `2026-09-25 14:30` | exactly that |
pub fn snooze_until<Tz: TimeZone>(
    phrase: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<DateTime<Utc>, String>
where
    Tz::Offset: std::fmt::Display,
{
    /// The hour a morning starts, for every phrase that means "a day" rather than "a time".
    const MORNING: u32 = 9;
    /// And an evening.
    const EVENING: u32 = 19;

    let here = now.with_timezone(zone);
    let phrase = phrase.trim().to_ascii_lowercase();
    let at = |day: chrono::NaiveDate, hour: u32| -> Result<DateTime<Utc>, String> {
        let naive = day
            .and_hms_opt(hour, 0, 0)
            .ok_or_else(|| format!("{hour}:00 is not a time"))?;
        // `earliest`: a local time can be skipped by a daylight-saving jump, in which case the
        // next valid instant is the honest answer rather than an error about clocks.
        zone.from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .ok_or_else(|| "that local time does not exist".to_owned())
    };

    if let Some(rest) = phrase.strip_prefix('+') {
        return relative(rest, now);
    }
    let today = here.date_naive();
    match phrase.as_str() {
        "later" => return Ok(now + TimeDelta::try_hours(3).unwrap_or_default()),
        "tonight" | "evening" => {
            // If the evening has already gone, the next one is tomorrow's — not one in the past.
            return if here.hour() < EVENING {
                at(today, EVENING)
            } else {
                at(today.succ_opt().ok_or("no tomorrow")?, EVENING)
            };
        }
        "tomorrow" => return at(today.succ_opt().ok_or("no tomorrow")?, MORNING),
        "weekend" | "saturday" | "sat" => return at(next_weekday(today, Weekday::Sat), MORNING),
        _ => {}
    }
    if let Some(day) = weekday_named(&phrase) {
        return at(next_weekday(today, day), MORNING);
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(&phrase, "%Y-%m-%d") {
        return at(date, MORNING);
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&phrase, "%Y-%m-%d %H:%M") {
        return zone
            .from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .ok_or_else(|| "that local time does not exist".to_owned());
    }
    Err(format!(
        "{phrase:?} is not a time I know. Try: later, tonight, tomorrow, weekend, \
         monday…sunday, +2h, +3d, 2026-09-25, or \"2026-09-25 14:30\""
    ))
}

/// `90m`, `2h`, `3d` — the part after a `+`.
fn relative(rest: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let rest = rest.trim();
    let (digits, unit) = rest.split_at(rest.len().saturating_sub(1));
    let count: i64 = digits
        .parse()
        .map_err(|_| format!("{rest:?} is not a number of minutes, hours or days"))?;
    if count <= 0 {
        return Err("a snooze goes forwards".to_owned());
    }
    let delta = match unit {
        "m" => TimeDelta::try_minutes(count),
        "h" => TimeDelta::try_hours(count),
        "d" => TimeDelta::try_days(count),
        _ => return Err(format!("{unit:?} is not m, h or d")),
    }
    .ok_or_else(|| format!("{rest:?} is longer than a mail client can wait"))?;
    Ok(now + delta)
}

fn weekday_named(name: &str) -> Option<Weekday> {
    Some(match name {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// The next `want` strictly after `from`. Never today: "monday" said on a Monday means the one
/// coming, not the hour that has already passed.
fn next_weekday(from: chrono::NaiveDate, want: Weekday) -> chrono::NaiveDate {
    let mut day = from;
    for _ in 0..7 {
        day = day.succ_opt().unwrap_or(day);
        if day.weekday() == want {
            return day;
        }
    }
    day
}

/// When a conversation comes back.
#[cfg(test)]
mod snoozing {
    use super::*;

    fn taipei() -> chrono::FixedOffset {
        chrono::FixedOffset::east_opt(8 * 3600).unwrap()
    }

    /// Tuesday 2026-09-22, 14:00 in Taipei (06:00 UTC).
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap()
    }

    /// What a phrase resolves to, written in the reader's zone.
    fn until(phrase: &str) -> String {
        snooze_until(phrase, now(), &taipei())
            .map(|t| {
                t.with_timezone(&taipei())
                    .format("%a %Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|e| format!("error: {e}"))
    }

    #[test]
    fn the_words_people_actually_use() {
        assert_eq!(until("later"), "Tue 2026-09-22 17:00");
        assert_eq!(until("tonight"), "Tue 2026-09-22 19:00");
        assert_eq!(until("tomorrow"), "Wed 2026-09-23 09:00");
        assert_eq!(until("weekend"), "Sat 2026-09-26 09:00");
        assert_eq!(until("friday"), "Fri 2026-09-25 09:00");
        // Case and surrounding space are not part of what was meant.
        assert_eq!(until("  ToMorrow "), "Wed 2026-09-23 09:00");
    }

    #[test]
    fn a_day_named_today_means_the_next_one_not_an_hour_that_has_passed() {
        // Said on a Tuesday afternoon, "tuesday" cannot mean this morning.
        assert_eq!(until("tuesday"), "Tue 2026-09-29 09:00");
    }

    #[test]
    fn tonight_after_the_evening_is_tomorrow_evening() {
        // 22:00 in Taipei, which is 14:00 UTC. The alternative is a snooze into the past, which
        // `Filter::SnoozeDue` would make due immediately — a button that appears to do nothing.
        let late = Utc.with_ymd_and_hms(2026, 9, 22, 14, 0, 0).unwrap();
        let got = snooze_until("tonight", late, &taipei()).unwrap();
        assert_eq!(
            got.with_timezone(&taipei())
                .format("%a %Y-%m-%d %H:%M")
                .to_string(),
            "Wed 2026-09-23 19:00"
        );
        assert!(got > late, "a snooze must be in the future");
    }

    #[test]
    fn offsets_and_dates() {
        assert_eq!(until("+90m"), "Tue 2026-09-22 15:30");
        assert_eq!(until("+2h"), "Tue 2026-09-22 16:00");
        assert_eq!(until("+3d"), "Fri 2026-09-25 14:00");
        assert_eq!(until("2026-12-25"), "Fri 2026-12-25 09:00");
        assert_eq!(until("2026-12-25 14:30"), "Fri 2026-12-25 14:30");
    }

    #[test]
    fn a_date_is_read_in_the_readers_zone_not_in_utc() {
        // 09:00 on Christmas morning in Taipei is 01:00 UTC. Read as UTC it would land at
        // 17:00 local — the afternoon of a day the user said "morning" about.
        let at = snooze_until("2026-12-25", now(), &taipei()).unwrap();
        assert_eq!(at.format("%Y-%m-%d %H:%M").to_string(), "2026-12-25 01:00");
        let in_utc = snooze_until("2026-12-25", now(), &Utc).unwrap();
        assert_eq!(
            in_utc.format("%Y-%m-%d %H:%M").to_string(),
            "2026-12-25 09:00"
        );
    }

    #[test]
    fn everything_it_returns_is_in_the_future() {
        // The property that matters: a snooze into the past is due the instant it is made, so
        // the conversation never leaves the inbox and the feature silently does nothing.
        for phrase in [
            "later",
            "tonight",
            "tomorrow",
            "weekend",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
            "+1m",
            "+1h",
            "+1d",
        ] {
            let at =
                snooze_until(phrase, now(), &taipei()).unwrap_or_else(|e| panic!("{phrase}: {e}"));
            assert!(at > now(), "{phrase} resolved to {at}, which is not later");
        }
    }

    #[test]
    fn a_phrase_it_does_not_know_says_what_it_does() {
        let why = snooze_until("next fortnight", now(), &taipei()).unwrap_err();
        assert!(why.contains("tomorrow") && why.contains("+2h"), "{why}");
        // Nonsense that looks like an offset, too.
        assert!(snooze_until("+", now(), &taipei()).is_err());
        assert!(snooze_until("+2y", now(), &taipei()).is_err());
        assert!(
            snooze_until("+0h", now(), &taipei()).is_err(),
            "zero is not forwards"
        );
        assert!(
            snooze_until("-2h", now(), &taipei()).is_err(),
            "nor is backwards"
        );
        assert!(snooze_until("", now(), &taipei()).is_err());
    }
}

/// Why the list pane is empty, which it never said.
#[cfg(test)]
mod nothing_tests {
    use super::*;

    #[test]
    fn no_account_beats_every_other_explanation() {
        // The first thing anyone sees. With no account there is nothing to search, so "nothing
        // matches" would send a new user hunting for a typo instead of doing the setup step.
        assert_eq!(nothing_to_show(0, ""), Nothing::NoAccount);
        assert_eq!(nothing_to_show(0, "invoice"), Nothing::NoAccount);
        assert_eq!(
            nothing_to_show(0, "").command(),
            Some("mailo account add <address>"),
            "the shell cannot add an account, so it has to name what can"
        );
    }

    #[test]
    fn a_search_that_matched_nothing_says_what_was_searched_for() {
        // The words are the thing most likely to be mistyped, so they go in the message.
        assert_eq!(
            nothing_to_show(1, "invoice"),
            Nothing::NoMatch("invoice".to_owned())
        );
        assert!(nothing_to_show(1, "invoice").message().contains("invoice"));
        assert_eq!(
            nothing_to_show(1, "  invoice  "),
            Nothing::NoMatch("invoice".to_owned()),
            "whitespace is not the search"
        );
    }

    #[test]
    fn an_empty_folder_is_ordinary_and_says_so_briefly() {
        assert_eq!(nothing_to_show(2, ""), Nothing::EmptyFolder);
        assert_eq!(nothing_to_show(2, "   "), Nothing::EmptyFolder);
        assert_eq!(nothing_to_show(2, "").message(), "Nothing here.");
        assert_eq!(nothing_to_show(2, "").command(), None);
    }
}

#[cfg(test)]
mod discarding {
    use super::*;

    fn composing() -> Composing {
        Composing {
            draft: DraftId::generate(),
            to: "ada@example.test".to_owned(),
            cc: String::new(),
            subject: "Re: lunch".to_owned(),
            body: "never mind".to_owned(),
            notice: None,
            confirming_discard: false,
        }
    }

    #[test]
    fn the_first_click_asks_and_the_second_deletes() {
        // Discard now removes the draft rather than closing the pane, and it sits beside Close.
        // One click must not be enough.
        let mut open = composing();
        assert_eq!(discard_click(Some(&open)), Some(Discarding::Confirm));
        open.confirming_discard = true;
        assert_eq!(
            discard_click(Some(&open)),
            Some(Discarding::Delete(open.draft))
        );
    }

    #[test]
    fn a_closed_composer_has_nothing_to_discard() {
        assert_eq!(discard_click(None), None);
    }

    #[test]
    fn reopening_the_composer_asks_again() {
        // The flag lives on the composer, so closing and reopening resets it. A confirmation
        // that survives the pane being closed is a trap set for the next draft.
        let draft = Draft {
            id: DraftId::generate(),
            account: AccountId::generate(),
            identity: IdentityId::generate(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Re: lunch".to_owned(),
            in_reply_to: None,
            forward_of: None,
            text: String::new(),
            html: None,
            attachments: vec![],
            state: SendState::Editing,
            updated: Utc.with_ymd_and_hms(2026, 9, 22, 0, 0, 0).unwrap(),
        };
        let mut shell = Shell::default();
        shell.compose(&draft);
        if let Some(c) = shell.composing.as_mut() {
            c.confirming_discard = true;
        }
        shell.close_composer();
        shell.compose(&draft);
        assert_eq!(
            discard_click(shell.composing.as_ref()),
            Some(Discarding::Confirm)
        );
    }
}

/// Dates, which were shown in UTC everywhere.
#[cfg(test)]
mod stamps {
    use super::*;
    use chrono::FixedOffset;

    fn taipei() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    /// 2026-09-22 09:02 in Taipei, which is 01:02 the same day in UTC.
    fn morning() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 1, 2, 0).unwrap()
    }

    #[test]
    fn an_instant_is_written_in_the_readers_zone_not_in_utc() {
        // The whole finding in one line: this read "09-22 01:02" for mail that arrived while
        // the user was having breakfast.
        assert_eq!(stamp(morning(), &taipei(), Stamp::Row), "09-22 09:02");
        assert_eq!(stamp(morning(), &taipei(), Stamp::Full), "2026-09-22 09:02");
    }

    #[test]
    fn an_evening_message_is_not_filed_under_tomorrow() {
        // 23:30 UTC is 07:30 the next morning in Taipei. Shown as UTC, every message that
        // arrived after 16:00 local carried the previous day's date — which is precisely the
        // state the application was in when this was found: the clock read the 22nd and every
        // row said the 21st.
        let evening = Utc.with_ymd_and_hms(2026, 9, 21, 23, 30, 0).unwrap();
        assert_eq!(stamp(evening, &taipei(), Stamp::Row), "09-22 07:30");
        assert_eq!(stamp(evening, &Utc, Stamp::Row), "09-21 23:30");
    }

    #[test]
    fn a_zone_behind_utc_rolls_the_other_way() {
        // Not "add eight hours somewhere". New York is five behind, so the same instant is the
        // day before, and a client that only ever shifted forward would be wrong here.
        let newyork = FixedOffset::west_opt(5 * 3600).unwrap();
        let just_after_midnight = Utc.with_ymd_and_hms(2026, 9, 22, 3, 15, 0).unwrap();
        assert_eq!(
            stamp(just_after_midnight, &newyork, Stamp::Row),
            "09-21 22:15"
        );
    }

    /// The clock reads 2026-09-22 14:00 in Taipei.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap()
    }

    #[test]
    fn todays_mail_shows_a_time_and_older_mail_does_not() {
        // The shell wrote "Sep 22" on a message that arrived an hour ago — the least useful
        // answer available, and the same one it gave for a message from three weeks back.
        let zone = taipei();
        assert_eq!(listed(morning(), now(), &zone), "09:02");
        // Yesterday evening in Taipei, which is a different calendar day and so not a time.
        let yesterday = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        assert_eq!(listed(yesterday, now(), &zone), "Mon");
        // Three weeks: past the weekday window, inside the year.
        let weeks_ago = Utc.with_ymd_and_hms(2026, 9, 1, 2, 0, 0).unwrap();
        assert_eq!(listed(weeks_ago, now(), &zone), "Sep 01");
        // Last year, where the year is the only part still worth printing.
        let old = Utc.with_ymd_and_hms(2024, 11, 14, 22, 13, 0).unwrap();
        assert_eq!(listed(old, now(), &zone), "2024-11-15");
    }

    #[test]
    fn today_is_a_calendar_day_not_the_last_twenty_four_hours() {
        // 00:10 this morning is today although it is fourteen hours ago; 23:50 last night is
        // not, although it is fourteen hours ago too. Elapsed-hours arithmetic gets both wrong.
        let zone = taipei();
        let just_after_midnight = Utc.with_ymd_and_hms(2026, 9, 21, 16, 10, 0).unwrap();
        assert_eq!(listed(just_after_midnight, now(), &zone), "00:10");
        let late_last_night = Utc.with_ymd_and_hms(2026, 9, 21, 15, 50, 0).unwrap();
        assert_eq!(listed(late_last_night, now(), &zone), "Mon");
    }

    #[test]
    fn the_list_stamp_is_in_the_readers_zone_too() {
        // The same instant is today in Taipei and yesterday in New York, so the two disagree
        // about which shape to use at all — not just about the digits.
        let newyork = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(listed(morning(), now(), &taipei()), "09:02");
        assert_eq!(listed(morning(), now(), &newyork), "Mon");
    }

    #[test]
    fn utc_is_left_alone() {
        // Someone whose machine is on UTC must see what was stored — this is the case that
        // made the bug invisible, because it is the one every test machine is in.
        assert_eq!(stamp(morning(), &Utc, Stamp::Full), "2026-09-22 01:02");
    }
}
