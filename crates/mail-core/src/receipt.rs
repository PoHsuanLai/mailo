//! Read receipts from the reader's side: whether a message is waiting for an answer, and giving
//! one.
//!
//! Never automatic. RFC 8098 §2.1 leaves the choice to the person reading, and a client that
//! answered on open would be telling every sender who asks — tracking pixels' older cousin —
//! that this mailbox exists and was read, and when. The only way a receipt leaves is
//! [`answer`] with [`ReceiptAnswer::Sent`], which a person asked for by name.

use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::{OriginalHeaders, ReceiptAsk, Reporting, ReturnPath};
use mail_store::{SqliteStore, Store};

/// Where a message stands on read receipts, for the reader to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptState {
    /// It does not ask, or it is our own message, or it is a receipt itself.
    NotAsked,
    /// Only its headers are here; whether it asks is known once the body arrives.
    Unknown,
    /// It asks, and nobody has answered. Show `ask.to`, and warn when `ask.return_path` is
    /// [`ReturnPath::Differs`].
    Pending(ReceiptAsk),
    /// Answered on this machine.
    Answered(ReceiptAnswer),
}

/// What [`answer`] reports on the `Reporting-UA` line.
fn agent() -> String {
    format!("mailo; mailo {}", env!("CARGO_PKG_VERSION"))
}

/// Where `message` stands. Reads the stored raw message, because the request is a header the
/// domain message does not carry.
pub fn state(store: &SqliteStore, message: &Message) -> Result<ReceiptState, String> {
    if let Some(answered) = store
        .receipt_answer(message.id)
        .map_err(|e| e.to_string())?
    {
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
    let bytes = store
        .blobs()
        .get(&store.connection(), raw)
        .map_err(|e| e.to_string())?;
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
) -> Result<String, String> {
    let original = store.message(message).map_err(|e| e.to_string())?;
    let ask = match state(store, &original)? {
        ReceiptState::Pending(ask) => ask,
        ReceiptState::Answered(ReceiptAnswer::Sent) => {
            return Err("a receipt for that message was already sent".to_owned());
        }
        ReceiptState::Answered(ReceiptAnswer::Declined) => {
            return Err("you already declined to send a receipt for that message".to_owned());
        }
        ReceiptState::Unknown => {
            return Err("only that message's headers are here yet; \
                        run mailo sync and try again"
                .to_owned());
        }
        ReceiptState::NotAsked => {
            return Err("that message did not ask for a read receipt".to_owned());
        }
    };

    let mut out = String::new();
    if answer == ReceiptAnswer::Sent {
        let raw = original
            .body
            .raw()
            .ok_or_else(|| "the message's body is missing".to_owned())?;
        let bytes = store
            .blobs()
            .get(&store.connection(), raw)
            .map_err(|e| e.to_string())?;
        let identity = crate::compose::identity_of(
            store,
            original.account,
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
            },
        )
        .map_err(|e| e.to_string())?;
        let frozen = store
            .blobs()
            .put(&store.connection(), &post.message)
            .map_err(|e| e.to_string())?;
        let queued = store
            .enqueue(
                original.account,
                RemoteIntent::Send {
                    draft: id,
                    raw: frozen,
                    mail_from: post.mail_from.clone(),
                    rcpt_to: post.rcpt_to.clone(),
                },
                &nothing_to_undo(),
                now,
            )
            .map_err(|e| e.to_string())?;
        if queued.is_none() {
            return Err("the receipt could not be queued".to_owned());
        }
        out.push_str(&format!(
            "queued a read receipt to {}\n\ndeliver it with: mailo sync\n",
            post.rcpt_to.join(", ")
        ));
    } else {
        let to: Vec<&str> = ask.to.iter().map(|a| a.email.as_str()).collect();
        out.push_str(&format!(
            "declined: no receipt will go to {}\n",
            to.join(", ")
        ));
    }

    store
        .answer_receipt(message, answer, now)
        .map_err(|e| e.to_string())?;
    // `None` is normal: a POP3 message has no server flags to set.
    store
        .enqueue(
            original.account,
            RemoteIntent::AddKeyword {
                messages: vec![message],
                keyword: Keyword::MdnSent,
            },
            &nothing_to_undo(),
            now,
        )
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// The lines `mailo show` prints under a message, or nothing.
pub fn describe(state: &ReceiptState, message: MessageId) -> String {
    match state {
        ReceiptState::NotAsked | ReceiptState::Unknown => String::new(),
        ReceiptState::Answered(ReceiptAnswer::Sent) => "    read receipt sent\n".to_owned(),
        ReceiptState::Answered(ReceiptAnswer::Declined) => "    read receipt declined\n".to_owned(),
        ReceiptState::Pending(ask) => {
            let to: Vec<&str> = ask.to.iter().map(|a| a.email.as_str()).collect();
            let mut out = format!(
                "    asks for a read receipt, to {}\n    \
                 send one with: mailo receipt {message}   or decline: mailo receipt {message} --decline\n",
                to.join(", ")
            );
            if let ReturnPath::Differs { return_path } = &ask.return_path {
                out.push_str(&format!(
                    "    careful: the receipt would go to another domain than the message came \
                     from ({return_path})\n"
                ));
            }
            out
        }
    }
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
    identities(store, message.account)
        .iter()
        .any(|(_, email)| email.eq_ignore_ascii_case(&message.from.email))
}

/// The identity the original was addressed to, when one of the account's is among its
/// recipients. `None` sends from the account's default.
fn addressed_identity(store: &SqliteStore, message: &Message) -> Option<IdentityId> {
    identities(store, message.account)
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
    let db = store.connection();
    let Ok(mut stmt) = db.prepare("SELECT id, from_email FROM identities WHERE account = ?1")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([account.to_string()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok)
        .filter_map(|(id, email)| Some((IdentityId::from_uuid(id.parse().ok()?), email)))
        .collect()
}
