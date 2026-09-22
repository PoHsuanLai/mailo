//! Raw bytes to a stored [`Message`].
//!
//! This is the seam the protocol crates deliberately cannot cross. A `Message` needs a
//! `MessageId`, a `ThreadId` and a stored `BlobId`, and none of the three is anything the wire
//! supplies — so `mail-proto` hands back `ProtoOutcome::Fetched { remote, raw }` and the work
//! of turning that into something the store will accept happens here, where the blob store,
//! `mail-mime` and the id space all exist.

use crate::RuntimeError;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// What arrived, and where from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    pub remote: RemoteRef,
    pub raw: Vec<u8>,
}

/// Turn fetched bytes into an [`Ingest`] the store can absorb.
///
/// `fallback_date` is the server's own idea of when a message arrived, used when the `Date`
/// header is missing or unparseable. That is common enough in real mail that treating it as an
/// error would reject ordinary messages; it is a fact to record, not a failure.
pub fn assemble(
    store: &SqliteStore,
    account: AccountId,
    mailbox: MailboxRef,
    cursor: Option<SyncCursor>,
    arrivals: Vec<Arrival>,
    fallback_date: DateTime<Utc>,
) -> Result<Ingest, RuntimeError> {
    let mut parsed = Vec::new();
    for arrival in arrivals {
        // A message we cannot parse is skipped, not fatal. One malformed message in a maildrop
        // of two thousand must not stop the other 1999 arriving.
        let Ok(fields) = mail_mime::parse(&arrival.raw) else {
            continue;
        };
        let blob = store
            .blobs()
            .put(&store.connection(), &arrival.raw)
            .map_err(RuntimeError::Store)?;
        parsed.push((arrival.remote, fields, blob));
    }

    // Thread them together, and onto conversations already stored. `existing` is what makes an
    // incremental sync join rather than renumber.
    let inputs: Vec<threading::ThreadInput<'_>> = parsed
        .iter()
        .map(|(_, fields, _)| threading::ThreadInput {
            message_id: fields.rfc_message_id.as_deref(),
            in_reply_to: fields.in_reply_to.as_deref(),
            references: &fields.references,
            subject: &fields.subject,
        })
        .collect();
    let known = |id: &str| thread_of_rfc_id(store, account, id);
    let threads = threading::thread(&inputs, &known);

    let mut messages = Vec::new();
    for ((remote, fields, blob), thread) in parsed.into_iter().zip(threads) {
        let key = message_key(&fields, &remote);
        let from = fields.from.clone().unwrap_or_else(|| Address {
            // A message with no From is malformed and still has to be listable; inventing a
            // plausible-looking sender would be worse than an obviously empty one.
            name: None,
            email: String::new(),
        });
        let attachments = fields
            .attachments
            .iter()
            .map(|part| {
                let stored = store
                    .blobs()
                    .put(&store.connection(), &part.bytes)
                    .map_err(RuntimeError::Store)?;
                Ok(Attachment {
                    name: part.name.clone(),
                    mime: part.mime.clone(),
                    size: part.bytes.len() as u64,
                    blob: stored,
                    inline: part.inline.clone(),
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;

        let message = Message {
            id: MessageId::generate(),
            thread,
            account,
            key: key.clone(),
            date: fields.date.unwrap_or(fallback_date),
            from,
            reply_to: fields.reply_to.clone(),
            to: fields.to.clone(),
            cc: fields.cc.clone(),
            bcc: fields.bcc.clone(),
            subject: fields.subject.clone(),
            in_reply_to: fields.in_reply_to.clone(),
            references: fields.references.clone(),
            rfc_message_id: fields.rfc_message_id.clone(),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Present {
                text: fields.text.clone(),
                raw: blob,
            },
            attachments,
        };
        messages.push(Fetched {
            remote,
            key,
            raw: blob,
            message,
        });
    }

    Ok(Ingest {
        mailbox,
        validity: UidValidity::Same,
        cursor,
        messages,
        flags: Vec::new(),
        labels: Vec::new(),
        label_names: Vec::new(),
        gone: Vec::new(),
    })
}

/// Headers only, for the pass that must not mark anything read.
///
/// Identical to [`assemble`] except the body is [`Body::Absent`] — which is why that variant
/// exists. A message listed from its headers is a real message, not a broken one.
pub fn assemble_headers(
    store: &SqliteStore,
    account: AccountId,
    mailbox: MailboxRef,
    cursor: Option<SyncCursor>,
    arrivals: Vec<Arrival>,
    fallback_date: DateTime<Utc>,
) -> Result<Ingest, RuntimeError> {
    let mut ingest = assemble(store, account, mailbox, cursor, arrivals, fallback_date)?;
    for fetched in &mut ingest.messages {
        fetched.message.body = Body::Absent;
    }
    Ok(ingest)
}

/// The identity a message is deduplicated by.
///
/// `Message-ID` where there is one. Where there is not — and real mail frequently has none —
/// a digest of the fields that do exist, so two copies of the same message still collapse
/// instead of arriving twice.
fn message_key(fields: &mail_mime::Parsed, remote: &RemoteRef) -> MessageKey {
    if let Some(id) = &fields.rfc_message_id {
        return MessageKey::Rfc(id.clone());
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(fields.subject.as_bytes());
    hasher.update(
        fields
            .from
            .as_ref()
            .map(|a| a.email.as_str())
            .unwrap_or("")
            .as_bytes(),
    );
    if let Some(date) = fields.date {
        hasher.update(date.to_rfc3339().as_bytes());
    }
    // The remote address is included only as a last resort tiebreak: without it, two genuinely
    // different messages with no Message-ID, the same subject, sender and timestamp would
    // collapse into one and the user would silently lose mail.
    match remote {
        RemoteRef::Pop { uidl } => hasher.update(uidl.as_bytes()),
        RemoteRef::Imap { mailbox, uid, .. } => {
            hasher.update(mailbox.as_bytes());
            hasher.update(&uid.to_le_bytes())
        }
    };
    MessageKey::Synthetic(*hasher.finalize().as_bytes())
}

/// The thread a stored message with this `Message-ID` already belongs to.
fn thread_of_rfc_id(store: &SqliteStore, account: AccountId, rfc_id: &str) -> Option<ThreadId> {
    let db = store.connection();
    let found: Option<String> = db
        .query_row(
            "SELECT thread FROM messages WHERE account = ?1 AND rfc_message_id = ?2 LIMIT 1",
            rusqlite::params![account.to_string(), rfc_id],
            |r| r.get(0),
        )
        .ok()?;
    found.and_then(|t| t.parse().ok()).map(ThreadId::from_uuid)
}

/// Absorb fetched bytes into the store, returning what changed.
pub fn absorb(
    store: &SqliteStore,
    account: AccountId,
    mailbox: MailboxRef,
    cursor: Option<SyncCursor>,
    arrivals: Vec<Arrival>,
    headers_only: bool,
    fallback_date: DateTime<Utc>,
) -> Result<Patch, RuntimeError> {
    let ingest = if headers_only {
        assemble_headers(store, account, mailbox, cursor, arrivals, fallback_date)?
    } else {
        assemble(store, account, mailbox, cursor, arrivals, fallback_date)?
    };
    store.ingest(account, ingest).map_err(RuntimeError::Store)
}
