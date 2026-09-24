//! Rules: what to do with mail as it arrives, and the reply sent while the user is away.
//!
//! A rule is a [`Filter`] and a list of [`RuleAction`]s. It is evaluated against **one message
//! at a time**, not a conversation: that is what a rule means to someone writing "mail from the
//! bank gets the Bills label", it is what a server's Sieve script sees at delivery, and it keeps
//! a reply arriving in an old thread from re-filing the messages that were already there.
//! [`one_message`] is how a `Filter` — a predicate over threads — reads a single message.
//!
//! Every clause of `Filter` can be written in a rule; how each one reads at arrival is said on
//! [`one_message`]. Which clauses a server can run is a separate question, answered where the
//! rules are compiled to Sieve.
//!
//! Rules are per account and ordered. Per account because every action names something that
//! lives on one account — a label, a folder, a server script — and because a server's Sieve
//! script is per account too; a rule for two accounts is two rules.

use crate::filter::{DateRange, Filter, MatchCtx};
use crate::id::{AccountId, RuleId};
use crate::message::{Message, ThreadSummary};
use crate::state::{Pin, Snooze};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A filter and what to do with the mail it matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: RuleId,
    pub account: AccountId,
    /// What the user calls it, unique on its account: the name `mailo rules` takes.
    pub name: String,
    /// Where it runs among the account's rules, lowest first. Ties break by name.
    pub position: u32,
    pub state: RuleState,
    pub filter: Filter,
    /// What to do, in order. A rule with none only matters for [`AfterMatch::Stop`].
    pub actions: Vec<RuleAction>,
    pub after: AfterMatch,
}

/// Whether a rule runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleState {
    Enabled,
    /// Kept, and skipped. Also left out of a server's script.
    Disabled,
}

/// What happens to the rules after this one, when this one matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterMatch {
    /// The next rule is tried too.
    Continue,
    /// No later rule sees this message. Sieve's `stop`.
    Stop,
}

/// One thing a rule does to a message it matched.
///
/// Labels and folders are named rather than referenced by id: a rule is written before the
/// label may exist, a server script names them as text, and a label the user deletes should not
/// silently turn a rule into one that does nothing. The name is resolved, and the label created
/// where it is new, when the rule acts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum RuleAction {
    /// Put this label on it.
    Label(String),
    Archive,
    Trash,
    Spam,
    /// Out of the inbox into the folder at this path — a label, where mailboxes are labels.
    File(String),
    MarkRead,
    Star,
}

impl Rule {
    /// Whether this rule acts on the message `ctx` describes: enabled, and its filter fits.
    pub fn fits(&self, ctx: &MatchCtx<'_>) -> bool {
        self.state == RuleState::Enabled && self.filter.fit(ctx)
    }
}

/// The rules that may act, in the order they run: enabled ones, by position then name.
///
/// Ordered here whatever order `rules` is in, so no caller can run them in an order the user
/// never chose. Running them is the caller's: each rule is tested against the message as the
/// rules before it left it, which only a caller holding the message between rules can do.
pub fn ordered(rules: &[Rule]) -> Vec<&Rule> {
    let mut out: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.state == RuleState::Enabled)
        .collect();
    out.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// How a [`Filter`] sees one message, when a rule asks about it.
///
/// The summary of a conversation holding that message alone, so each clause reads the message's
/// own value:
///
/// - `From`, `To`, `Subject`, `Date`, `HasAttachment`: its own sender, `To` and `Cc`, subject,
///   date and attachments.
/// - `Text`: its subject, addresses and whatever body text is held (pass it as the corpus). A
///   message whose body has not been fetched yet is searched by its headers alone.
/// - `Read`, `Starred`, `HasLabel`: what the server said when it arrived (Gmail's labels), or
///   what earlier rules have done to it since, which is how a later rule sees an earlier one.
/// - `InMailbox`: the role it arrived as or is filed as now.
/// - `InFolder`: its own server addresses, where it is still filed there (`Store::placed` gives
///   them); a copy of the thread elsewhere does not put this message in that folder.
/// - `Account`: its account.
/// - `Snoozed`, `SnoozeDue`, `Pinned`: never true. They are this client's state about a
///   conversation, and a message on its own has none; a rule that needs them cannot fire.
pub fn one_message(message: &Message) -> ThreadSummary {
    ThreadSummary::derive(
        message.thread,
        std::slice::from_ref(message),
        Snooze::Inactive,
        Pin::Unpinned,
    )
}

/// The reply sent to mail arriving while the user is away (RFC 5230).
///
/// Only ever run by a server: a reply that needs this client to be running is one that stops
/// the moment the laptop is closed, which is exactly when it is wanted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vacation {
    pub account: AccountId,
    pub subject: String,
    /// The reply's text.
    pub body: String,
    /// How long before the same sender is answered again (`:days`), at least one.
    pub days: u16,
    /// The user's own addresses (`:addresses`): mail sent to one of these is answered, and mail
    /// that names none of them — a list, a Bcc to someone else — is not.
    pub addresses: Vec<String>,
    /// The address the reply is from (`:from`), where it should not be the one written to.
    pub from: Option<String>,
    /// When it applies, half-open like every [`DateRange`]. Unbounded ends are open.
    pub during: DateRange,
}

impl Vacation {
    /// `:days` when none is given, the RFC's own suggestion.
    pub const DEFAULT_DAYS: u16 = 7;

    /// Whether the reply should be going out at `now`.
    pub fn active_at(&self, now: DateTime<Utc>) -> bool {
        self.during.from.is_none_or(|from| now >= from) && self.during.to.is_none_or(|to| now < to)
    }
}
