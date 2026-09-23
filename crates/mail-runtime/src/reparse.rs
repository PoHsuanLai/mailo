//! Rewriting stored messages whose headers an older parser misread.
//!
//! Migration 0012 queues every message held whole whose subject or addresses carry U+FFFD — the
//! mark of raw 8-bit headers, or multi-byte encoded-words, read as UTF-8. Their bytes are in the
//! blob store, so fixing them needs no network: parse again, write back.

use crate::RuntimeError;
use mail_domain::{Change, ChangeId, Message, MessageId, Patch};
use mail_store::{SqliteStore, Store};

/// Re-read each queued message from its stored bytes and rewrite what its headers say. Returns
/// how many were rewritten.
///
/// Only the fields a header parse yields change — subject, sender and recipients. Flags,
/// mailbox, labels, thread and body are the store's own and are written back as they are, so
/// nothing the user did to a message is undone by its being re-read.
///
/// A message whose bytes are gone or no longer parse leaves the queue unchanged: another try
/// would get the same answer, and a queue that never empties is a cost on every start.
pub fn reparse_queued(store: &SqliteStore) -> Result<usize, RuntimeError> {
    let mut rewritten = 0;
    for id in store.reparse_queue()? {
        if let Some(message) = reparsed(store, id) {
            store.apply(
                message.account,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )?;
            rewritten += 1;
        }
        store.reparsed(id)?;
    }
    Ok(rewritten)
}

/// `id` with its header fields read again, or `None` when there is nothing to read them from.
fn reparsed(store: &SqliteStore, id: MessageId) -> Option<Message> {
    let message = store.message(id).ok()?;
    let raw = message.body.raw()?;
    let bytes = store.blobs().get(&store.connection(), raw).ok()?;
    let fields = mail_mime::parse(&bytes).ok()?;
    let from = fields.from.unwrap_or_else(|| message.from.clone());
    Some(Message {
        subject: fields.subject,
        from,
        reply_to: fields.reply_to,
        to: fields.to,
        cc: fields.cc,
        bcc: fields.bcc,
        ..message
    })
}
