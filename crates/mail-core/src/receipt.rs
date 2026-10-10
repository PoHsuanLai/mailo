//! Read receipts from the reader's side: whether a message is waiting for an answer, and giving
//! one.
//!
//! Never automatic. RFC 8098 §2.1 leaves the choice to the person reading, and a client that
//! answered on open would be telling every sender who asks — tracking pixels' older cousin —
//! that this mailbox exists and was read, and when. The only way a receipt leaves is
//! [`answer`] with [`ReceiptAnswer::Sent`], which a person asked for by name.

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::{Human, OriginalHeaders, ReceiptAsk, Reporting, Words};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// Where a message stands on read receipts, for the reader to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptState {
    /// It does not ask, or it is our own message, or it is a receipt itself.
    NotAsked,
    /// Only its headers are here; whether it asks is known once the body arrives.
    Unknown,
    /// It asks, and nobody has answered. Show `ask.to`, and warn when `ask.return_path` is
    /// [`mail_mime::ReturnPath::Differs`].
    Pending(ReceiptAsk),
    /// Answered on this machine.
    Answered(ReceiptAnswer),
}

/// What [`answer`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// A receipt is queued for these envelope recipients, and leaves on the next sync.
    Sent { to: Vec<String> },
    /// The request was declined; no receipt goes to these addresses.
    Declined { to: Vec<String> },
}

/// What [`answer`] reports on the `Reporting-UA` line.
fn agent() -> String {
    format!("mailo; mailo {}", env!("CARGO_PKG_VERSION"))
}

/// The receipt's sentences. Says what a receipt means and, as plainly, what it does not.
///
/// Broken into short lines by hand, so an ordinary receipt stays 7-bit text rather than
/// quoted-printable that a person reading the source has to decode.
fn words() -> Words {
    fn body(human: &Human<'_>) -> String {
        let mut out = format!(
            "This is a receipt for the message you sent to {}",
            human.reader
        );
        if let Some(date) = &human.date {
            out.push_str(&format!("\r\non {date}"));
        }
        if human.subject.is_empty() {
            out.push_str(" with no subject.\r\n");
        } else {
            out.push_str(&format!(" with the subject\r\n\"{}\".\r\n", human.subject));
        }
        out.push_str(
            "\r\nIt was displayed on the recipient's screen. That says nothing\r\n\
             about whether it was read, understood or agreed with.\r\n",
        );
        out
    }
    Words {
        subject_prefix: "Read: ",
        body,
    }
}

/// Where `message` stands. Reads the stored raw message, because the request is a header the
/// domain message does not carry.
pub fn state(store: &SqliteStore, message: &Message) -> Result<ReceiptState, CoreError> {
    if let Some(answered) = store.receipt_answer(message.id)? {
        return Ok(ReceiptState::Answered(answered));
    }
    if matches!(message.mailbox, MailboxRole::Sent | MailboxRole::Drafts) || is_ours(store, message)
    {
        // A copy of our own message carries our own request, which is not for us to answer.
        return Ok(ReceiptState::NotAsked);
    }
    let Some(raw) = message.body.raw() else {
        return Ok(ReceiptState::Unknown);
    };
    let bytes = store.blobs().get(raw)?;
    Ok(match mail_mime::receipt_asked(&bytes) {
        Some(ask) => ReceiptState::Pending(ask),
        None => ReceiptState::NotAsked,
    })
}

/// Answer `message`'s request: queue the receipt, or decline it. Either way the answer is kept,
/// so the question is not put again, and `$MDNSent` is queued for the server so other clients
/// do not put it either (RFC 3503).
///
/// The receipt leaves from the account that received the original, as the identity it was
/// addressed to when there is one, and through the outbox like any other message: nothing here
/// touches the network, and the next sync delivers it.
pub fn answer(
    store: &SqliteStore,
    message: MessageId,
    answer: ReceiptAnswer,
    now: DateTime<Utc>,
) -> Result<Settled, CoreError> {
    let original = store.message(message)?;
    let ask = match state(store, &original)? {
        ReceiptState::Pending(ask) => ask,
        ReceiptState::Answered(ReceiptAnswer::Sent) => {
            return Err(CoreError::ReceiptSent);
        }
        ReceiptState::Answered(ReceiptAnswer::Declined) => {
            return Err(CoreError::ReceiptDeclined);
        }
        ReceiptState::Unknown => {
            return Err(CoreError::HeadersOnly);
        }
        ReceiptState::NotAsked => {
            return Err(CoreError::NoReceiptAsked);
        }
    };

    let settled = if answer == ReceiptAnswer::Sent {
        let raw = original.body.raw().ok_or(CoreError::BodyMissing)?;
        let bytes = store.blobs().get(raw)?;
        let identity = crate::compose::identity_of(
            store,
            original.account.clone(),
            addressed_identity(store, &original),
        )?;
        // Named like a draft because the outbox's submission is keyed by one; no draft row
        // exists, which the drain already treats as "nothing to mark".
        let id = DraftId::generate();
        let post = mail_mime::receipt(
            &bytes,
            &Reporting {
                reader: &identity,
                agent: &agent(),
                id,
                at: now,
                headers: OriginalHeaders::Included,
                words: words(),
            },
        )?;
        let frozen = store.blobs().put(&post.message)?;
        let queued = store.enqueue(
            original.account.clone(),
            RemoteIntent::Send {
                draft: id,
                raw: frozen,
                mail_from: post.mail_from.clone(),
                rcpt_to: post.rcpt_to.clone(),
            },
            &nothing_to_undo(),
            now,
        )?;
        if queued.is_none() {
            return Err(CoreError::ReceiptNotQueued);
        }
        Settled::Sent { to: post.rcpt_to }
    } else {
        Settled::Declined {
            to: ask.to.iter().map(|a| a.email.clone()).collect(),
        }
    };

    store.answer_receipt(message, answer, now)?;
    // `None` is normal: a POP3 message has no server flags to set.
    store.enqueue(
        original.account,
        RemoteIntent::AddKeyword {
            messages: vec![message],
            keyword: Keyword::MdnSent,
        },
        &nothing_to_undo(),
        now,
    )?;
    Ok(settled)
}

fn nothing_to_undo() -> Patch {
    // Neither a receipt that left nor an answer given can be taken back by a patch.
    Patch {
        id: ChangeId::generate(),
        changes: Vec::new(),
    }
}

/// Whether `message` was written by this account: its sender is one of the account's identities.
fn is_ours(store: &SqliteStore, message: &Message) -> bool {
    identities(store, message.account.clone())
        .iter()
        .any(|(_, email)| email.eq_ignore_ascii_case(&message.from.email))
}

/// The identity the original was addressed to, when one of the account's is among its
/// recipients. `None` sends from the account's default.
fn addressed_identity(store: &SqliteStore, message: &Message) -> Option<IdentityId> {
    identities(store, message.account.clone())
        .into_iter()
        .find(|(_, email)| {
            message
                .to
                .iter()
                .chain(&message.cc)
                .any(|addr| addr.email.eq_ignore_ascii_case(email))
        })
        .map(|(id, _)| id)
}

fn identities(store: &SqliteStore, account: AccountId) -> Vec<(IdentityId, String)> {
    store
        .identities(account)
        .unwrap_or_default()
        .into_iter()
        .map(|identity| (identity.id, identity.from.email))
        .collect()
}
