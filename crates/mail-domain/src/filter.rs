//! The one query language. The UI, saved views and search all build a [`Filter`]; nothing
//! else queries the store.

use crate::content::Address;
use crate::id::{AccountId, LabelId};
use crate::message::ThreadSummary;
use crate::remote::MailboxRef;
use crate::state::{Attachments, MailboxRole, Pin, ReadState, Snooze, Star};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod fold;
mod tokens;
pub use tokens::search_tokens;

/// How a text clause matches.
///
/// **What these mean depends on the clause holding them**, because the two clause families are
/// backed by different machinery: [`Filter::From`], [`Filter::To`] and [`Filter::Subject`]
/// compare one stored value (SQL `LIKE`), while [`Filter::Text`] queries a tokenized full-text
/// index (SQLite FTS5). A substring query cannot use that index, so `Filter::Text` reads these
/// as word and phrase queries instead. See [`Filter::Text`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum TextMatch {
    /// Substring of the value, ASCII-case-insensitive.
    ///
    /// In [`Filter::Text`]: every word of the needle occurs, as a whole word, somewhere in the
    /// thread's text.
    Contains(String),
    /// The whole value, ASCII-case-insensitive.
    ///
    /// In [`Filter::Text`]: the needle's words occur adjacently and in order within one field —
    /// a phrase query.
    Exact(String),
}

/// A half-open interval, `[from, to)`. Both ends optional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DateRange {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// A predicate over threads.
///
/// Full-text search is [`Filter::Text`] rather than a sibling field, so that
/// `Or(Text(..), From(..))` is expressible at all.
///
/// The leaf variants are *predicates*, not mirrors of the state enums: `Snoozed`, not
/// `Snooze(Snooze)`, because matching against an exact `Until(instant)` is never what anyone
/// wants; `HasAttachment`, not `Attachments(Present { count })`, which would have meant
/// "exactly N attachments".
///
/// Only absolute instants appear here. Relative dates ("last 7 days") are resolved by the UI
/// before the filter is built, so a saved view means the same thing when it is reloaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Filter {
    /// Matches everything. A unified inbox is this, with no `Account` clause.
    All,
    /// Matches nothing. The identity for `Or`.
    Nothing,
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
    Account(AccountId),
    /// True when the thread has any message in `role`.
    InMailbox(MailboxRole),
    Read(ReadState),
    Starred(Star),
    HasLabel(LabelId),
    /// True when one of the thread's messages is held in this mailbox **and still filed there
    /// here**. All three of:
    ///
    /// - it has a server address in the mailbox: a `remote_map` row for the account and the
    ///   path exactly as the server spells it;
    /// - its role is what one of its addresses is filed as ([`crate::FolderRoles::filed_as`]),
    ///   so a copy held in two folders is listed in both, archiving it from the inbox leaves it
    ///   in the other folder as the server does, and trashing or marking it spam here takes it
    ///   out of every folder at once (see [`Filed`]);
    /// - no move of it into another folder ([`crate::Op::File`]) is still queued.
    ///
    /// The address is server truth and is never changed optimistically — it is what the
    /// outbox addresses the message by. The other two are the local intent laid over it, the
    /// way [`Filter::InMailbox`] carries it: a message moved out of a folder view leaves the
    /// listing the moment it is moved, and comes back if the server refuses the move and its
    /// undo is applied. [`Placed`] is that per-message fact, as the store hands it in.
    ///
    /// On an account whose folders are labels (Gmail), [`Filter::HasLabel`] is how a folder
    /// is listed: only `INBOX` and `Sent` are fetched there, so no other path has addresses.
    InFolder(MailboxRef),
    From(TextMatch),
    To(TextMatch),
    Subject(TextMatch),
    /// Full text: subject, participants, snippet, and the plain-text body.
    ///
    /// **This clause matches words, not substrings**, unlike every other [`TextMatch`] clause
    /// here. It is answered by SQLite's FTS5 index, which stores tokens; `LIKE '%needle%'`
    /// cannot use that index, so making the pure function promise substrings would promise
    /// something the store cannot deliver at any acceptable cost. Concretely:
    ///
    /// - [`TextMatch::Contains`] is a **word co-occurrence** query. The needle is tokenized,
    ///   and every one of its tokens must appear as a whole word somewhere in the searched
    ///   text. `Contains("xamp")` does **not** match `example.com`; `Contains("example")`
    ///   does. Tokens need not be adjacent, in order, or even in the same field:
    ///   `Contains("ada lunch")` matches a thread Ada sent with `lunch` in the subject.
    /// - [`TextMatch::Exact`] is a **phrase** query: the needle's tokens, adjacent and in
    ///   order, inside a single field. It is not whole-value equality — against a tokenized
    ///   index of message bodies that would only ever be useful for a one-word subject, and it
    ///   is not expressible as an FTS5 query at all. Phrase is the strictest thing this index
    ///   can honestly answer, and it keeps `Exact` stronger than `Contains`.
    /// - A needle with **no tokens** (empty, or punctuation only) matches **nothing**, for
    ///   both. This is the one place where `Contains("")` is not vacuously true: an FTS5 query
    ///   with no terms cannot be expressed, so there is nothing for the SQL side to agree with.
    /// - Matching folds **case and diacritics** — `resume` matches `Résumé` — because that is
    ///   what `unicode61 remove_diacritics 2` does. The other clauses fold ASCII case only.
    ///   That asymmetry is deliberate; the reason is on `text_fits` in this module.
    Text(TextMatch),
    Date(DateRange),
    HasAttachment,
    /// Snoozed to some future instant.
    Snoozed,
    /// Snoozed, and the instant has passed — the thread is due back in the inbox.
    SnoozeDue,
    Pinned,
}

/// Everything [`Filter::fit`] needs in order to decide.
///
/// `corpus` is the full searchable text of the thread, exactly as the store indexes it: every
/// message's subject, sender name and address, and body — not only the newest, and not only the
/// body. It cannot be derived from `summary`, because a [`ThreadSummary`] carries the *oldest*
/// message's subject and the *newest* message's sender, while a thread is searchable through
/// all of them.
///
/// Supplying too little here is a silent parity bug rather than an error: `fit` simply fails to
/// match something the SQL side finds, and search quietly misses a message. A caller that has
/// no corpus passes `None`, and [`Filter::Text`] then sees only the summary fields.
///
/// `folders` is every server address any of the thread's messages has, each with what
/// [`Filter::InFolder`] weighs beside it. A summary does not carry it — it is `remote_map` and
/// the outbox, not message state — so it is handed in beside the summary, like the corpus. A
/// caller that does not have it passes `&[]`, and then no `InFolder` clause matches.
#[derive(Debug, Clone, Copy)]
pub struct MatchCtx<'a> {
    pub summary: &'a ThreadSummary,
    pub corpus: Option<&'a str>,
    pub folders: &'a [Placed],
    pub now: DateTime<Utc>,
}

/// One message's server address, with what decides whether the message is still filed there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// Where the server holds it.
    pub mailbox: MailboxRef,
    pub filed: Filed,
    pub leaving: Leaving,
}

/// Whether a message is filed here as one of the places the server holds it is filed.
///
/// A property of the message, not of one address: a copy held in `INBOX` and `Projects` and
/// filed in the inbox is filed as held, and is listed in both folders, because the server lists
/// it in both. Archived here, it is still filed as `Projects` is, and stays there — which is
/// what the server does with an archive, which moves the inbox copy only. Trashed or marked
/// spam, it is filed as neither, and leaves both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filed {
    /// Its role is [`crate::FolderRoles::filed_as`] of one of its addresses' paths.
    AsHeld,
    /// Filed somewhere none of its addresses is: moved here, and the server not yet caught up.
    Away,
}

/// Whether a move of a message into another folder is queued and not yet settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Leaving {
    Staying,
    /// A [`crate::Op::File`] into a folder other than this address's is in the outbox.
    Elsewhere,
}

impl Placed {
    /// Every address of one message filed as `role` here, on an account whose folders `roles`
    /// names, with `moving` the target folders of the `File` moves still queued for it.
    pub fn of_message(
        role: MailboxRole,
        addresses: Vec<MailboxRef>,
        roles: &crate::FolderRoles,
        moving: &[String],
    ) -> Vec<Placed> {
        let filed = if addresses.iter().any(|m| roles.filed_as(&m.path) == role) {
            Filed::AsHeld
        } else {
            Filed::Away
        };
        addresses
            .into_iter()
            .map(|mailbox| {
                let leaving = if moving.iter().any(|to| *to != mailbox.path) {
                    Leaving::Elsewhere
                } else {
                    Leaving::Staying
                };
                Placed {
                    mailbox,
                    filed,
                    leaving,
                }
            })
            .collect()
    }

    /// Whether the message is listed in this mailbox: held there, filed as held, not leaving.
    pub fn filed_here(&self) -> bool {
        self.filed == Filed::AsHeld && self.leaving == Leaving::Staying
    }
}

impl Filter {
    /// Whether this thread matches.
    ///
    /// Returns `bool`, not an enum. The no-`bool` rule is about *state fields*, where a named
    /// variant documents meaning; a predicate's return is neither, and wrapping it costs
    /// `&&`, `!` and `Iterator::filter` at every call site.
    ///
    /// **This function and `mail-store`'s SQL compiler are two implementations of one
    /// semantics and will diverge.** `MemoryStore` is implemented by calling this, and a
    /// proptest asserts `fit(f, ctx) == (id ∈ sql(f))`. If that test is skipped, the bug
    /// ships and presents as "search silently misses a message".
    pub fn fit(&self, ctx: &MatchCtx<'_>) -> bool {
        let s = ctx.summary;
        match self {
            // `And(vec![])` is true and `Or(vec![])` is false: the identities of the two
            // folds, so that `And(xs ++ ys) == And(xs) && And(ys)` holds for empty parts.
            Filter::All => true,
            Filter::Nothing => false,
            Filter::And(fs) => fs.iter().all(|f| f.fit(ctx)),
            Filter::Or(fs) => fs.iter().any(|f| f.fit(ctx)),
            Filter::Not(f) => !f.fit(ctx),
            Filter::Account(id) => s.account == *id,
            Filter::InMailbox(role) => s.mailboxes.contains(*role),
            Filter::Read(state) => s.read == *state,
            Filter::Starred(star) => s.star == *star,
            Filter::HasLabel(label) => s.labels.contains(label),
            Filter::InFolder(mailbox) => ctx
                .folders
                .iter()
                .any(|placed| placed.mailbox == *mailbox && placed.filed_here()),
            Filter::From(m) => address_fits(m, &s.from),
            Filter::To(m) => to_fits(m, s),
            Filter::Subject(m) => text_fits(m, &s.subject),
            Filter::Text(m) => text_search_fits(m, ctx),
            Filter::Date(range) => {
                range.from.is_none_or(|from| s.last_date >= from)
                    && range.to.is_none_or(|to| s.last_date < to)
            }
            Filter::HasAttachment => matches!(s.attachments, Attachments::Present { .. }),
            Filter::Snoozed => matches!(s.snooze, Snooze::Until(_)),
            // Implies `Snoozed`: a due thread is still a snoozed thread.
            Filter::SnoozeDue => matches!(s.snooze, Snooze::Until(at) if at <= ctx.now),
            Filter::Pinned => matches!(s.pin, Pin::Rank(_)),
        }
    }
}

/// Whether `haystack` satisfies `m` as a substring or whole-value match. Used by
/// [`Filter::From`], [`Filter::To`] and [`Filter::Subject`] — **not** by [`Filter::Text`].
///
/// Case folding here is **ASCII-only**: `Contains("RE")` matches `"re: hi"`, but `Exact("É")`
/// does not match `"é"` and `Contains("STRASSE")` does not match `"straße"`.
///
/// **This is the opposite of [`Filter::Text`], which folds diacritics too, and the asymmetry is
/// intended** — it will otherwise read as a bug. The two clause families are answered by
/// different machinery in the store and can only promise what that machinery does:
/// `From`/`To`/`Subject` compile to SQL `LIKE`, whose `upper`/`lower` are ASCII-only unless
/// SQLite is built with ICU, while `Text` compiles to an FTS5 query whose
/// `unicode61 remove_diacritics 2` tokenizer folds both. Aligning them is a change to make on
/// both sides at once, with the parity proptest to prove it — not a one-line fix here.
fn text_fits(m: &TextMatch, haystack: &str) -> bool {
    match m {
        // `Contains("")` matches everything, as `str::contains` does.
        TextMatch::Contains(needle) => haystack
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase()),
        TextMatch::Exact(value) => haystack.eq_ignore_ascii_case(value),
    }
}

/// Whether an address satisfies `m`, matching the display name and the email address
/// separately — so `Exact` means "this whole name" or "this whole address", never the
/// rendered `Name <email>` form, which no store column holds.
fn address_fits(m: &TextMatch, addr: &Address) -> bool {
    text_fits(m, &addr.email) || addr.name.as_deref().is_some_and(|name| text_fits(m, name))
}

/// Whether the thread's full-text corpus satisfies `m`, as a word or phrase query.
///
/// The semantics, and why they are not substring semantics, are documented on [`Filter::Text`].
/// This is the half of the pair that has to agree with `messages_fts`; it is written to be
/// read beside the SQL compiler, not beside [`text_fits`].
fn text_search_fits(m: &TextMatch, ctx: &MatchCtx<'_>) -> bool {
    let (TextMatch::Contains(raw) | TextMatch::Exact(raw)) = m;
    let needle = search_tokens(raw);
    // No terms, no query. Not vacuously true: FTS5 cannot express an empty MATCH, so a
    // vacuous truth here would be a divergence the SQL side could not reproduce.
    if needle.is_empty() {
        return false;
    }
    let fields = text_corpus(ctx);
    match m {
        // Co-occurrence, not adjacency, and not confined to one field: `a AND b` in FTS5.
        TextMatch::Contains(_) => {
            let haystack: Vec<String> = fields.iter().flat_map(|f| search_tokens(f)).collect();
            needle.iter().all(|token| haystack.contains(token))
        }
        // Adjacency within one field: `"a b"` in FTS5.
        TextMatch::Exact(_) => fields.iter().any(|f| {
            search_tokens(f)
                .windows(needle.len())
                .any(|run| run == needle.as_slice())
        }),
    }
}

/// The fields [`Filter::Text`] searches.
///
/// `participants` is every sender in the thread, so a thread stays findable by an older
/// sender that `summary.from` has scrolled past — and it is also what the store's per-message
/// FTS rows hold, which is what the parity proptest compares against. The snippet is a prefix
/// of the newest body: including it can only find what a full body search would have found,
/// never something else, so it is safe to search when `corpus` is `None`.
fn text_corpus<'a>(ctx: &MatchCtx<'a>) -> Vec<&'a str> {
    let s = ctx.summary;
    let mut fields = Vec::with_capacity(4 + 2 * (s.participants.len() + s.recipients.len()));
    fields.push(s.subject.as_str());
    fields.push(s.snippet.as_str());
    push_address(&mut fields, &s.from);
    for participant in &s.participants {
        push_address(&mut fields, participant);
    }
    // Who it was addressed to, as well as who wrote it: the store indexes each message's `To`
    // and `Cc`, and this is the union of those. Never `Bcc` — `ThreadSummary` leaves it out.
    for recipient in &s.recipients {
        push_address(&mut fields, recipient);
    }
    fields.extend(ctx.corpus);
    fields
}

fn push_address<'a>(fields: &mut Vec<&'a str>, addr: &'a Address) {
    fields.push(addr.email.as_str());
    if let Some(name) = addr.name.as_deref() {
        fields.push(name);
    }
}

/// Whether a [`Filter::To`] clause matches: whether anyone the thread was addressed to fits.
///
/// Against `recipients`, never `participants`. Matching senders would answer "did this person
/// write here?" rather than "was this addressed to them?", and fire on threads the user was
/// never a recipient of. The store compiles `To` against `thread_summary.recipients`, the same
/// union, which is what the parity proptest holds it to.
fn to_fits(m: &TextMatch, summary: &ThreadSummary) -> bool {
    summary.recipients.iter().any(|a| address_fits(m, a))
}
