//! Messages, threads, and the derivation between them.

use crate::content::{Address, Attachment, Body};
use crate::id::{AccountId, LabelId, MessageId, ThreadId};
use crate::state::{Attachments, MailboxRole, MailboxSet, Pin, ReadState, Snooze, Star};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How we recognise the *same* message arriving again under a different [`crate::RemoteRef`].
///
/// Deduplication key, not an address. Two `RemoteRef`s with one `MessageKey` are one message
/// in two mailboxes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum MessageKey {
    /// The `Message-ID` header, normalized: angle brackets stripped, ASCII-lowercased.
    Rfc(String),
    /// `X-GM-MSGID`. Stable across mailboxes on Gmail, which is exactly the case
    /// [`crate::RemoteRef`] cannot express.
    Gmail(u64),
    /// No usable `Message-ID`: a digest of date, from, subject and the first bytes of the
    /// body. Mail in the wild is frequently this broken.
    ///
    /// Persisted as 64 hex characters rather than a 32-element JSON array, which would cost
    /// ~100 bytes per key and be unreadable in a SQLite browser.
    Synthetic(#[serde(with = "hex32")] [u8; 32]),
}

/// Serde helper: a 32-byte digest as lowercase hex.
mod hex32 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    use std::fmt::Write as _;

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(64);
        for b in bytes {
            let _ = write!(out, "{b:02x}");
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let text = String::deserialize(d)?;
        let raw = text.as_bytes();
        if raw.len() != 64 {
            return Err(D::Error::custom(format!(
                "expected 64 hex characters, got {}",
                raw.len()
            )));
        }
        let mut out = [0u8; 32];
        for (slot, pair) in out.iter_mut().zip(raw.chunks_exact(2)) {
            let text = std::str::from_utf8(pair).map_err(D::Error::custom)?;
            *slot = u8::from_str_radix(text, 16).map_err(D::Error::custom)?;
        }
        Ok(out)
    }
}

/// One message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub thread: ThreadId,
    pub account: AccountId,
    pub key: MessageKey,
    pub date: DateTime<Utc>,
    pub from: Address,
    /// The `Reply-To` header; empty when the sender omitted it, in which case a reply goes to
    /// `from`.
    ///
    /// Load-bearing, not decoration: mailing lists and no-reply senders redirect replies here.
    /// A client that ignores it sends every list reply to the wrong address.
    #[serde(default)]
    pub reply_to: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    /// The `In-Reply-To` header, normalized like [`MessageKey::Rfc`].
    pub in_reply_to: Option<String>,
    /// The `References` header, oldest first, normalized like [`MessageKey::Rfc`].
    pub references: Vec<String>,
    /// This message's own `Message-ID`, normalized. Absent when the sender omitted it.
    pub rfc_message_id: Option<String>,
    pub read: ReadState,
    pub star: Star,
    /// A single role: an individual message really is in one place.
    pub mailbox: MailboxRole,
    pub labels: Vec<LabelId>,
    pub body: Body,
    pub attachments: Vec<Attachment>,
}

/// The row shown in a list. Every field marked derived is a function of the thread's
/// messages; see [`ThreadSummary::derive`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub account: AccountId,
    /// Derived: the subject of the oldest message, with reply prefixes intact.
    pub subject: String,
    /// Derived: a preview of the newest message.
    pub snippet: String,
    /// Derived: the sender of the newest message.
    pub from: Address,
    /// Derived: everyone who appeared as a sender, oldest first, deduplicated.
    pub participants: Vec<Address>,
    /// Derived: everyone addressed on any message — `To` and `Cc` — oldest first,
    /// deduplicated.
    ///
    /// `Bcc` is deliberately excluded. A blind copy is not a fact about the thread that its
    /// other readers share, and surfacing it in a list row would leak it.
    #[serde(default)]
    pub recipients: Vec<Address>,
    /// Derived: the newest message's date.
    pub last_date: DateTime<Utc>,
    pub message_count: u32,
    /// Derived: `Unread` if *any* message is unread.
    pub read: ReadState,
    /// Derived: `Starred` if *any* message is starred.
    pub star: Star,
    /// Derived: the union over messages. A thread is not in one place.
    pub mailboxes: MailboxSet,
    /// Derived: the union over messages, deduplicated.
    pub labels: Vec<LabelId>,
    /// Derived: the total across messages.
    pub attachments: Attachments,
    /// Thread-level, not derived: set by the user.
    pub snooze: Snooze,
    /// Thread-level, not derived: set by the user.
    pub pin: Pin,
}

impl ThreadSummary {
    /// Recompute every derived field from the thread's messages.
    ///
    /// Named and public because applying an [`crate::Op`] to a [`crate::Target::Threads`]
    /// fans out to each message and then has to rebuild the summary — so an op needs the
    /// thread *and* its messages, not "a loaded row".
    ///
    /// `messages` must be non-empty and must all belong to `id`. `snooze` and `pin` are
    /// carried through unchanged because they are thread-level user state.
    pub fn derive(id: ThreadId, messages: &[Message], snooze: Snooze, pin: Pin) -> ThreadSummary {
        // Caller invariant, per CONVENTIONS.md section 5: an empty thread is programmer error,
        // not malformed mail. There is no honest summary for it -- subject, sender, date and
        // account would all have to be invented, and a fabricated row is worse in every list
        // than a loud failure at the call site that produced it.
        assert!(
            !messages.is_empty(),
            "ThreadSummary::derive requires at least one message; a thread with none has no \
             subject, sender or date to derive"
        );

        // Oldest first, ties broken by position in `messages`, so that every derived field is
        // a total, stable function of the input: two messages stamped with the same second
        // must not reorder the participant list between two calls.
        let mut order: Vec<&Message> = messages.iter().collect();
        order.sort_by_key(|m| m.date);
        let oldest = order[0]; // non-empty: asserted above
        let newest = order[order.len() - 1]; // non-empty: asserted above

        let mut participants: Vec<Address> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut recipients: Vec<Address> = Vec::new();
        let mut seen_rcpt: Vec<String> = Vec::new();
        let mut labels: Vec<LabelId> = Vec::new();
        let mut read = ReadState::Read;
        let mut star = Star::Unstarred;
        let mut attachments: usize = 0;

        for m in &order {
            // Addresses are compared case-insensitively: `Ada@Example.com` and
            // `ada@example.com` are one participant, and the first spelling seen wins because
            // it carries the display name the sender used first.
            let key = m.from.email.to_lowercase();
            if !seen.contains(&key) {
                seen.push(key);
                participants.push(m.from.clone());
            }
            // To and Cc only. Bcc stays out: see the field's doc comment.
            for addr in m.to.iter().chain(m.cc.iter()) {
                let key = addr.email.to_lowercase();
                if !seen_rcpt.contains(&key) {
                    seen_rcpt.push(key);
                    recipients.push(addr.clone());
                }
            }
            for label in &m.labels {
                if !labels.contains(label) {
                    labels.push(*label);
                }
            }
            if m.read == ReadState::Unread {
                read = ReadState::Unread;
            }
            if m.star == Star::Starred {
                star = Star::Starred;
            }
            attachments = attachments.saturating_add(m.attachments.len());
        }

        ThreadSummary {
            id,
            // Every message of a thread belongs to one account; threads are not merged across
            // accounts in v1.
            account: newest.account,
            subject: oldest.subject.clone(),
            snippet: snippet_of(newest.body.text()),
            from: newest.from.clone(),
            participants,
            recipients,
            last_date: newest.date,
            message_count: u32::try_from(messages.len()).unwrap_or(u32::MAX),
            read,
            star,
            mailboxes: order.iter().map(|m| m.mailbox).collect(),
            labels,
            attachments: Attachments::of(u32::try_from(attachments).unwrap_or(u32::MAX)),
            snooze,
            pin,
        }
    }
}

/// The most characters a snippet may hold. Enough for two lines in a list row; short enough
/// that a summary stays cheap to store beside every thread.
const SNIPPET_CHARS: usize = 140;

/// The preview rule, in one place so it is documented rather than folded into `derive`.
///
/// Every run of whitespace -- spaces, tabs, the newlines of a quoted reply -- collapses to a
/// single space, leading and trailing whitespace is dropped, and the result is cut to the
/// first [`SNIPPET_CHARS`] characters (`char`s, never bytes, so the cut cannot split a
/// multi-byte character). No ellipsis is appended: whether a truncated preview ends in one is
/// the renderer's decision, and baking it in would make the stored snippet lie about its own
/// length. A message with no `text/plain` part yields an empty snippet rather than a guess
/// scraped out of HTML -- extracting text from HTML is `mail-mime`'s job, not the domain's.
fn snippet_of(text: Option<&str>) -> String {
    let Some(text) = text else {
        return String::new();
    };

    let mut out = String::new();
    let mut taken = 0usize;
    for word in text.split_whitespace() {
        if taken == SNIPPET_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
            taken += 1;
        }
        for ch in word.chars() {
            if taken == SNIPPET_CHARS {
                break;
            }
            out.push(ch);
            taken += 1;
        }
    }
    // Only ever one, from the separator pushed just as the budget ran out.
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// A conversation and the messages in it, newest last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub summary: ThreadSummary,
    pub messages: Vec<MessageId>,
}
