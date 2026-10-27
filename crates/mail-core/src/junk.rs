//! A junk filter of this client's own, as Mail has: it learns from what the user calls spam and
//! not spam, and judges new mail in the inbox by the words it has learned.
//!
//! Three questions, kept apart, as notifications keep theirs:
//!
//! - **What a message's words say** is [`score`]: Robinson's way of combining each word's
//!   spamminess (Fisher's method over the fifteen words that say most), a pure function of the
//!   counts. Nothing is said until the filter has learned [`ENOUGH`] of each kind.
//! - **Whether it is judged at all** is [`spared`]: never mail from someone in Contacts, someone
//!   the user has written to, or mail addressed to the user's full name, as Mail spares them.
//! - **What is done** is the [`Policy`]'s: moved to Spam, or left where it is with a Junk label.
//!   The server's own word (`X-Spam-Flag: YES`, `X-Spam-Status: Yes`) counts as spam when the
//!   policy trusts it, and mail the server filed in its own Junk is learned from, not judged.
//!
//! The counts live in the store, per account (`junk_words`); learning is [`learn`], which the
//! window's Spam and Not Spam call ([`crate::act::perform`]).

use crate::error::{CoreError, Logged};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, Body, LabelOrigin, MailboxRole, Membership, Message, MessageId, Op,
};
use mail_store::junk::{Judged, JunkCounts, Learned};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// Whether the filter judges new mail at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filtering {
    #[default]
    On,
    Off,
}

/// What is done with mail judged junk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Treat {
    /// Into Spam, as the user's own Spam would put it.
    #[default]
    Move,
    /// Left where it is, with the [`JUNK_LABEL`] on it.
    Mark,
}

/// Whether one of the reasons to spare a sender, or to trust the server, applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Heed {
    #[default]
    Yes,
    No,
}

/// When the filter runs against the user's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// After them: a rule that files a message has the last word over it.
    #[default]
    AfterRules,
    /// Before them: junk is out of the inbox before any rule sees it.
    BeforeRules,
}

/// How the filter behaves: Settings' Junk Mail section, as mail-app hands it over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Policy {
    pub filtering: Filtering,
    pub treat: Treat,
    /// Spare a sender in Contacts.
    pub contacts: Heed,
    /// Spare a sender the user has written to.
    pub written_to: Heed,
    /// Spare mail addressed to the user's full name.
    pub full_name: Heed,
    /// Take the server's `X-Spam-Flag` and `X-Spam-Status` as its verdict.
    pub server_says: Heed,
    pub order: Order,
}

/// The label a [`Treat::Mark`] puts on junk.
pub const JUNK_LABEL: &str = "Junk";

/// Messages of each kind the filter must have learned before it judges anything: until then
/// its counts are an accident of the first few messages.
pub const ENOUGH: u64 = 5;

/// How sure the filter must be before it calls a message junk.
pub const THRESHOLD: f64 = 0.9;

/// How many of a message's words decide: the ones furthest from neutral.
const TELLING: usize = 15;

/// The distinct words a message is judged by: its subject's and its text's, folded to lower
/// case, three to twenty letters or digits long, and its sender's domain as one word.
pub fn words(message: &Message) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |word: String| {
        if !out.contains(&word) {
            out.push(word);
        }
    };
    if let Some((_, domain)) = message.from.email.rsplit_once('@') {
        add(format!("from:{}", domain.to_ascii_lowercase()));
    }
    let text = match &message.body {
        Body::Present { text, .. } => text.as_deref().unwrap_or(""),
        Body::Absent => "",
    };
    for source in [message.subject.as_str(), text] {
        for word in source.split(|c: char| !c.is_alphanumeric()) {
            let count = word.chars().count();
            if (3..=20).contains(&count) && !word.chars().all(|c| c.is_ascii_digit()) {
                add(word.to_lowercase());
            }
        }
    }
    out
}

/// How likely a message with `words` is junk, from `counts`: near 1 junk, near 0 not, 0.5 when
/// the words say nothing. `None` until the filter has learned [`ENOUGH`] of each kind.
pub fn score(counts: &JunkCounts) -> Option<f64> {
    if counts.spam < ENOUGH || counts.ham < ENOUGH {
        return None;
    }
    let (spams, hams) = (counts.spam as f64, counts.ham as f64);
    // Robinson's f(w): the word's own spamminess, pulled towards a half by how little it has
    // been seen.
    let mut each: Vec<f64> = counts
        .words
        .iter()
        .map(|(_, spam, ham)| {
            let (b, g) = (*spam as f64 / spams, *ham as f64 / hams);
            let seen = (*spam + *ham) as f64;
            let p = if b + g == 0.0 { 0.5 } else { b / (b + g) };
            ((0.5 + seen * p) / (1.0 + seen)).clamp(0.01, 0.99)
        })
        .collect();
    if each.is_empty() {
        return Some(0.5);
    }
    each.sort_by(|a, b| (b - 0.5).abs().total_cmp(&(a - 0.5).abs()));
    each.truncate(TELLING);
    let n = each.len();
    let h = chi_squared_survival(-2.0 * each.iter().map(|f| f.ln()).sum::<f64>(), 2 * n);
    let s = chi_squared_survival(
        -2.0 * each.iter().map(|f| (1.0 - f).ln()).sum::<f64>(),
        2 * n,
    );
    Some((1.0 + h - s) / 2.0)
}

/// The probability a chi-squared variable with `freedom` (even) degrees exceeds `x`.
fn chi_squared_survival(x: f64, freedom: usize) -> f64 {
    let m = x / 2.0;
    let mut term = (-m).exp();
    let mut sum = term;
    for i in 1..freedom / 2 {
        term *= m / i as f64;
        sum += term;
    }
    sum.min(1.0)
}

/// Whether `message` is spared judging, by whom it is from or to: `names` are the user's full
/// names, from their identities.
pub fn spared(store: &SqliteStore, message: &Message, policy: &Policy, names: &[String]) -> bool {
    let contact = store
        .contact(&message.from.email)
        .or_log("a sender could not be looked up")
        .flatten();
    if let Some(contact) = &contact {
        if policy.contacts == Heed::Yes && contact.origin != mail_store::Origin::History {
            return true;
        }
        if policy.written_to == Heed::Yes && contact.written.count > 0 {
            return true;
        }
    }
    if policy.full_name == Heed::Yes
        && message.to.iter().chain(&message.cc).any(|to| {
            to.name.as_deref().is_some_and(|name| {
                names
                    .iter()
                    .any(|mine| mine.eq_ignore_ascii_case(name.trim()))
            })
        })
    {
        return true;
    }
    false
}

/// Whether the server that delivered `raw` said it is spam.
pub fn server_says_spam(raw: &[u8]) -> bool {
    let flag = mail_mime::archive::header(raw, "X-Spam-Flag")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("yes"));
    let status = mail_mime::archive::header(raw, "X-Spam-Status").is_some_and(|value| {
        value
            .trim()
            .split([',', ' ', ';'])
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case("yes"))
    });
    flag || status
}

/// Learn `messages` as junk or not: the window's Spam and Not Spam. Each is counted once, and
/// one learned the other way before is taken back first.
pub fn learn(
    store: &SqliteStore,
    messages: &[Message],
    judged: Judged,
) -> Result<usize, CoreError> {
    let mut learned = 0;
    for message in messages {
        let said =
            store.junk_learn(message.account.clone(), message.id, &words(message), judged)?;
        if said != Learned::Unchanged {
            learned += 1;
        }
    }
    Ok(learned)
}

/// What sifting a pass's arrivals did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sifted {
    /// Judged junk and moved or marked.
    pub junk: Vec<MessageId>,
}

/// Judge what a pass stored for the first time on `account`, as `policy` says.
///
/// Mail the server put in its own Junk is learned as spam rather than judged: that is the
/// server's filter speaking. Only mail in the inbox is judged, so a rule that filed it, or the
/// user who moved it already, has the last word.
pub fn sift(
    store: &SqliteStore,
    account: AccountId,
    caps: &AccountCaps,
    arrived: &[MessageId],
    policy: &Policy,
    now: DateTime<Utc>,
) -> Result<Sifted, CoreError> {
    let mut sifted = Sifted::default();
    if policy.filtering == Filtering::Off || arrived.is_empty() {
        return Ok(sifted);
    }
    let names: Vec<String> = store
        .identities(account.clone())?
        .into_iter()
        .filter_map(|identity| identity.from.name)
        .collect();
    for id in arrived {
        let Ok(message) = store.message(*id) else {
            continue;
        };
        if message.account != account {
            continue;
        }
        match message.mailbox {
            MailboxRole::Spam => {
                learn(store, std::slice::from_ref(&message), Judged::Spam)?;
                continue;
            }
            MailboxRole::Inbox => {}
            _ => continue,
        }
        if spared(store, &message, policy, &names) {
            continue;
        }
        let server = policy.server_says == Heed::Yes
            && match &message.body {
                Body::Present { raw, .. } => store
                    .blobs()
                    .get(*raw)
                    .is_ok_and(|bytes| server_says_spam(&bytes)),
                Body::Absent => false,
            };
        let judged = server || {
            let counts = store.junk_counts(account.clone(), &words(&message))?;
            score(&counts).is_some_and(|p| p >= THRESHOLD)
        };
        if !judged {
            continue;
        }
        let op = match policy.treat {
            Treat::Move => Op::Spam,
            Treat::Mark => Op::Label(
                mail_store::rules::label_named(
                    store,
                    account.clone(),
                    JUNK_LABEL,
                    LabelOrigin::User,
                )?,
                Membership::In,
            ),
        };
        mail_store::rules::act_on(store, caps, &op, &message, now)?;
        sifted.junk.push(message.id);
    }
    Ok(sifted)
}

#[cfg(test)]
mod tests;
