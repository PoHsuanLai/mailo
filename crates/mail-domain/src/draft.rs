//! Composition. Local-first: a draft is persisted before it is ever uploaded or sent, so an
//! offline draft is never lost.

use crate::content::Address;
use crate::id::{AccountId, BlobId, DraftId, IdentityId, MessageId};
use crate::message::Message;
use crate::retry::Retry;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A file the user attached, already read into the blob store.
///
/// A [`BlobId`], never a path: reading the file is I/O and must happen in `mail-runtime`
/// *before* the pure domain op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAttachment {
    pub name: String,
    pub mime: String,
    pub blob: BlobId,
}

/// Where a draft is in its lifecycle. Visible in the UI: "Sending…", "Failed — retry".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum SendState {
    Editing,
    Queued,
    Sending,
    Failed {
        reason: String,
        retry: Retry,
    },
    Sent {
        at: DateTime<Utc>,
        /// The stored copy, once it comes back from the Sent folder. `None` until then, and
        /// permanently `None` on POP3, which has no Sent folder to read.
        message: Option<MessageId>,
    },
}

/// A message being composed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub id: DraftId,
    pub account: AccountId,
    pub identity: IdentityId,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    /// Set when this is a reply, so `In-Reply-To` and `References` can be built correctly.
    pub in_reply_to: Option<MessageId>,
    pub forward_of: Option<MessageId>,
    pub text: String,
    pub html: Option<String>,
    pub attachments: Vec<PendingAttachment>,
    pub state: SendState,
    pub updated: DateTime<Utc>,
}

/// Who a reply goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyScope {
    /// The sender alone, honouring `Reply-To`.
    Sender,
    /// Everyone on the original except our own identity.
    All,
}

impl Draft {
    /// A reply to `msg`.
    ///
    /// Sets `in_reply_to` and inherits `References`, so the reply threads correctly for the
    /// recipient as well as for us.
    ///
    /// `Reply-To` wins over `From` when the sender set one (RFC 5322 §3.6.2), which is what
    /// routes a list reply to the list rather than to whoever happened to post.
    ///
    /// `text` is left empty: no quoted original. Quoting is a *rendering* decision — the
    /// `>`-prefix depth, the attribution line's wording and locale, whether to quote the
    /// HTML part at all — and doing it here would bake one answer into persisted draft rows
    /// that the composer could never revisit.
    pub fn reply_to(
        msg: &Message,
        identity: &crate::account::Identity,
        scope: ReplyScope,
        now: DateTime<Utc>,
    ) -> Draft {
        // `Reply-To` when present, else `From`. Under `ReplyScope::All` the original sender
        // is kept as well, so a list reply still reaches the person who wrote it.
        let mut to = if msg.reply_to.is_empty() {
            vec![msg.from.clone()]
        } else {
            msg.reply_to.clone()
        };
        let mut cc = Vec::new();
        if scope == ReplyScope::All {
            // The original recipients join `to`; the original `Cc` stays `Cc`, which is what
            // every other client does and what the recipients expect to see.
            to.push(msg.from.clone());
            to.extend(msg.to.iter().cloned());
            cc.extend(msg.cc.iter().cloned());
        }
        // `Bcc` is never carried over: the original's blind recipients were blind for a
        // reason, and on a received message the field is our own address anyway.
        dedup_addresses(&mut to, identity);
        dedup_addresses(&mut cc, identity);
        cc.retain(|addr| !to.iter().any(|kept| same_mailbox(kept, addr)));

        Draft {
            id: DraftId::generate(),
            account: identity.account,
            identity: identity.id,
            to,
            cc,
            bcc: Vec::new(),
            subject: prefixed(&msg.subject, "Re: ", &["re:"]),
            in_reply_to: Some(msg.id),
            forward_of: None,
            text: String::new(),
            html: None,
            attachments: Vec::new(),
            state: SendState::Editing,
            updated: now,
        }
    }

    /// A forward of `msg`, with no recipients filled in.
    ///
    /// `text` is left empty for the same reason as [`Draft::reply_to`]: the forwarded
    /// original is composed at render time, not frozen into the row. `attachments` is empty
    /// too — re-attaching the original's parts means reading blobs, which is I/O and belongs
    /// in `mail-runtime` before it hands us a [`PendingAttachment`].
    pub fn forward_of(
        msg: &Message,
        identity: &crate::account::Identity,
        now: DateTime<Utc>,
    ) -> Draft {
        Draft {
            id: DraftId::generate(),
            account: identity.account,
            identity: identity.id,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            // `Fw:` counts as an existing forward prefix: Outlook writes it and stacking
            // `Fwd: Fw:` on top reads as noise.
            subject: prefixed(&msg.subject, "Fwd: ", &["fwd:", "fw:"]),
            in_reply_to: None,
            forward_of: Some(msg.id),
            text: String::new(),
            html: None,
            attachments: Vec::new(),
            state: SendState::Editing,
            updated: now,
        }
    }
}

/// `subject` with `prefix` prepended, unless it already carries one of `existing`.
///
/// Comparison is ASCII case-insensitive and ignores leading whitespace, so `"RE: hi"`,
/// `"re:hi"` and `"  Re: hi"` all count as already-prefixed and nothing stacks. Only the
/// English prefixes are recognised: a localised `"AW:"` or `"Re :"` gains a `"Re: "`, which
/// is ugly but never loses information, and matching every locale's prefix list is how a
/// subject that merely *starts* with a word gets silently truncated.
fn prefixed(subject: &str, prefix: &str, existing: &[&str]) -> String {
    let head = subject.trim_start();
    // Compared as bytes, never as a `&str` slice: `head[..p.len()]` panics on a subject whose
    // third byte lands inside a multi-byte character, and subjects are attacker-supplied.
    let head = head.as_bytes();
    if existing
        .iter()
        .any(|p| head.len() >= p.len() && head[..p.len()].eq_ignore_ascii_case(p.as_bytes()))
    {
        subject.to_owned()
    } else {
        format!("{prefix}{subject}")
    }
}

/// Whether two addresses are the same mailbox. The display name is ignored and the email is
/// compared ASCII case-insensitively — the domain is case-insensitive by RFC 5321 and, while
/// the local part is not, no mail system in practice treats `A@x` and `a@x` as two people.
fn same_mailbox(a: &Address, b: &Address) -> bool {
    a.email.eq_ignore_ascii_case(&b.email)
}

/// Drop our own addresses and any repeats, keeping the first occurrence of each mailbox.
///
/// "Our own" is the identity's `from` *and* its `reply_to`: if replies to us are directed to
/// an alias, that alias is us too, and a reply-all that includes it mails ourselves.
fn dedup_addresses(addresses: &mut Vec<Address>, identity: &crate::account::Identity) {
    let mut kept: Vec<Address> = Vec::with_capacity(addresses.len());
    for addr in addresses.drain(..) {
        let ours = same_mailbox(&addr, &identity.from)
            || identity
                .reply_to
                .as_ref()
                .is_some_and(|own| same_mailbox(&addr, own));
        if ours || kept.iter().any(|k| same_mailbox(k, &addr)) {
            continue;
        }
        kept.push(addr);
    }
    *addresses = kept;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Identity;
    use crate::content::Body;
    use crate::id::{BlobId, ThreadId};
    use crate::message::MessageKey;
    use crate::state::{IsDefault, MailboxRole, ReadState, Star};

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
            .expect("literal is valid RFC 3339")
            .with_timezone(&Utc)
    }

    fn addr(email: &str) -> Address {
        Address {
            name: None,
            email: email.to_owned(),
        }
    }

    fn identity() -> Identity {
        Identity {
            id: IdentityId::generate(),
            account: AccountId::generate(),
            from: addr("me@example.test"),
            reply_to: None,
            signature: None,
            default: IsDefault::Default,
        }
    }

    fn message(subject: &str) -> Message {
        Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: AccountId::generate(),
            key: MessageKey::Rfc("original@example.test".to_owned()),
            date: at(),
            from: addr("sender@example.test"),
            reply_to: Vec::new(),
            to: vec![addr("me@example.test"), addr("other@example.test")],
            cc: vec![addr("cc@example.test")],
            bcc: vec![addr("bcc@example.test")],
            subject: subject.to_owned(),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: Some("original@example.test".to_owned()),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Present {
                text: Some("hello".to_owned()),
                raw: BlobId::generate(),
            },
            attachments: Vec::new(),
        }
    }

    #[test]
    fn reply_goes_to_reply_to_when_the_sender_set_one() {
        // The mailing-list case: List-Post redirects replies away from the individual who
        // happened to post. Replying to `from` here is the bug F8 was about.
        let mut msg = message("Lunch");
        msg.reply_to = vec![addr("list@example.test")];
        let me = identity();

        let sender = Draft::reply_to(&msg, &me, ReplyScope::Sender, at());
        assert_eq!(
            sender.to,
            vec![addr("list@example.test")],
            "Reply-To wins over From (RFC 5322 3.6.2)"
        );

        // Reply-all still reaches the person who actually wrote it.
        let all = Draft::reply_to(&msg, &me, ReplyScope::All, at());
        assert!(all.to.iter().any(|a| a.email == "list@example.test"));
        assert!(
            all.to.iter().any(|a| a.email == "sender@example.test"),
            "reply-all must keep the original author, not only the list"
        );
    }

    #[test]
    fn reply_to_sender_only() {
        let msg = message("Lunch");
        let me = identity();
        let draft = Draft::reply_to(&msg, &me, ReplyScope::Sender, at());
        assert_eq!(draft.to, vec![addr("sender@example.test")]);
        assert!(draft.cc.is_empty());
        assert!(draft.bcc.is_empty());
        assert_eq!(draft.subject, "Re: Lunch");
        assert_eq!(draft.in_reply_to, Some(msg.id));
        assert_eq!(draft.forward_of, None);
        assert_eq!(draft.state, SendState::Editing);
        assert_eq!(draft.updated, at());
        assert_eq!(draft.identity, me.id);
        assert_eq!(draft.account, me.account);
        assert!(draft.text.is_empty());
    }

    #[test]
    fn reply_all_drops_us_and_bcc() {
        let msg = message("Lunch");
        let me = identity();
        let draft = Draft::reply_to(&msg, &me, ReplyScope::All, at());
        assert_eq!(
            draft.to,
            vec![addr("sender@example.test"), addr("other@example.test")]
        );
        assert_eq!(draft.cc, vec![addr("cc@example.test")]);
        assert!(draft.bcc.is_empty());
    }

    #[test]
    fn reply_all_drops_our_alias_and_repeats() {
        let mut msg = message("Lunch");
        msg.to = vec![
            addr("ME@Example.TEST"),
            addr("alias@example.test"),
            addr("other@example.test"),
        ];
        msg.cc = vec![addr("OTHER@example.test"), addr("cc@example.test")];
        let mut me = identity();
        me.reply_to = Some(addr("alias@example.test"));
        let draft = Draft::reply_to(&msg, &me, ReplyScope::All, at());
        assert_eq!(
            draft.to,
            vec![addr("sender@example.test"), addr("other@example.test")]
        );
        // `other` is already in `to`, so it is not repeated in `cc`.
        assert_eq!(draft.cc, vec![addr("cc@example.test")]);
    }

    #[test]
    fn subject_prefixes_do_not_stack() {
        const CASES: &[(&str, &str, &str)] = &[
            ("Lunch", "Re: Lunch", "Fwd: Lunch"),
            ("Re: Lunch", "Re: Lunch", "Fwd: Re: Lunch"),
            ("RE: Lunch", "RE: Lunch", "Fwd: RE: Lunch"),
            ("re:Lunch", "re:Lunch", "Fwd: re:Lunch"),
            ("  Re: Lunch", "  Re: Lunch", "Fwd:   Re: Lunch"),
            ("Fwd: Lunch", "Re: Fwd: Lunch", "Fwd: Lunch"),
            ("FW: Lunch", "Re: FW: Lunch", "FW: Lunch"),
            ("Reunion", "Re: Reunion", "Fwd: Reunion"),
            ("", "Re: ", "Fwd: "),
        ];
        let me = identity();
        for (subject, reply, forward) in CASES {
            let msg = message(subject);
            assert_eq!(
                Draft::reply_to(&msg, &me, ReplyScope::Sender, at()).subject,
                *reply,
                "reply to {subject:?}"
            );
            assert_eq!(
                Draft::forward_of(&msg, &me, at()).subject,
                *forward,
                "forward of {subject:?}"
            );
        }
    }

    #[test]
    fn forward_has_no_recipients() {
        let msg = message("Lunch");
        let me = identity();
        let draft = Draft::forward_of(&msg, &me, at());
        assert!(draft.to.is_empty());
        assert!(draft.cc.is_empty());
        assert!(draft.bcc.is_empty());
        assert_eq!(draft.in_reply_to, None);
        assert_eq!(draft.forward_of, Some(msg.id));
        assert_eq!(draft.state, SendState::Editing);
        assert!(draft.attachments.is_empty());
    }

    #[test]
    fn drafts_get_distinct_ids() {
        let msg = message("Lunch");
        let me = identity();
        let a = Draft::reply_to(&msg, &me, ReplyScope::Sender, at());
        let b = Draft::reply_to(&msg, &me, ReplyScope::Sender, at());
        assert_ne!(a.id, b.id);
    }
}
