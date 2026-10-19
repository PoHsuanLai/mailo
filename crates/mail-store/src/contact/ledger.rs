//! What mail teaches the address book, written once for both stores.
//!
//! The policy is: spam and drafts count as nothing; a message counts once; the Sent copy of a
//! submission already counted is recognised by its fingerprint and keeps only the `Own` event;
//! a submission whose draft is gone is learned from its envelope. A store supplies the rows
//! those rules read and write as a [`Ledger`], and nothing here knows which kind it is.

use super::Kind;
use super::learn::{self, Event, Seen};
use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{Address, BlobId, Draft, MailboxRole, Message, MessageId};
use porter_core::AccountId;

/// The rows the learning policy reads and writes.
pub(crate) trait Ledger {
    /// Mark message `id` counted, and say whether it was not before.
    fn first_count(&mut self, id: MessageId) -> Result<bool, StoreError>;

    /// The start of a message's raw bytes, where this store holds them.
    fn head(&self, raw: BlobId) -> Option<Vec<u8>>;

    /// Forget one submission with this fingerprint, and say whether there was one.
    fn take_print(&mut self, print: &str) -> Result<bool, StoreError>;

    /// Remember a confirmed submission's fingerprint, so its Sent copy is not counted again.
    fn keep_print(&mut self, print: &str, at: DateTime<Utc>) -> Result<(), StoreError>;

    /// Apply each event to its address's row.
    fn hear(
        &mut self,
        account: Option<AccountId>,
        events: &[(String, Event)],
    ) -> Result<(), StoreError>;
}

/// Whether message `id` is one the book has yet to count, marking it counted if so. Spam and
/// drafts are never counted, and never marked.
pub(crate) fn first_sighting(
    ledger: &mut impl Ledger,
    id: MessageId,
    mailbox: MailboxRole,
) -> Result<bool, StoreError> {
    if matches!(mailbox, MailboxRole::Spam | MailboxRole::Drafts) {
        return Ok(false);
    }
    ledger.first_count(id)
}

/// Whether a received message's raw bytes, where held, say it came through a list. Bytes that
/// are not held say nothing, rather than stopping an ingest over a hint.
pub(crate) fn sender_of(ledger: &impl Ledger, mailbox: MailboxRole, raw: Option<BlobId>) -> Kind {
    let received = matches!(
        mailbox,
        MailboxRole::Inbox | MailboxRole::Archive | MailboxRole::Trash
    );
    match raw.filter(|_| received).and_then(|raw| ledger.head(raw)) {
        Some(head) if learn::listed(&head) => Kind::Bulk,
        _ => Kind::Person,
    }
}

/// What one ingested message teaches the book, once per message.
pub(crate) fn learn_fetched(
    ledger: &mut impl Ledger,
    account: AccountId,
    id: MessageId,
    message: &Message,
    raw: Option<BlobId>,
) -> Result<(), StoreError> {
    // Asked first, so a body arriving for a message already counted costs one insert and not a
    // read of its bytes.
    if !first_sighting(ledger, id, message.mailbox)? {
        return Ok(());
    }
    let seen = Seen {
        mailbox: message.mailbox,
        date: message.date,
        from: &message.from,
        to: &message.to,
        cc: &message.cc,
        bcc: &message.bcc,
        subject: &message.subject,
        sender: sender_of(ledger, message.mailbox, raw),
    };
    learn_seen(ledger, account, &seen)
}

/// Count a message seen for the first time, recognising the Sent copy of a submission already
/// counted.
pub(crate) fn learn_seen(
    ledger: &mut impl Ledger,
    account: AccountId,
    seen: &Seen<'_>,
) -> Result<(), StoreError> {
    let mut events = learn::events(seen);
    if seen.mailbox == MailboxRole::Sent
        && ledger.take_print(&learn::fingerprint([seen.to, seen.cc], seen.subject))?
    {
        events.retain(|(_, e)| *e == Event::Own);
    }
    ledger.hear(Some(account), &events)
}

/// Count a confirmed submission: from the draft where it still exists, else from the envelope.
pub(crate) fn learn_sent(
    ledger: &mut impl Ledger,
    account: AccountId,
    draft: Option<&Draft>,
    mail_from: &str,
    rcpt_to: &[String],
    at: DateTime<Utc>,
) -> Result<(), StoreError> {
    let envelope: Vec<Address> = rcpt_to
        .iter()
        .map(|email| Address {
            name: None,
            email: email.clone(),
        })
        .collect();
    let (to, cc, bcc, subject) = match draft {
        Some(d) => (&d.to[..], &d.cc[..], &d.bcc[..], d.subject.as_str()),
        None => (&envelope[..], &[][..], &[][..], ""),
    };
    let from = Address {
        name: None,
        email: mail_from.to_owned(),
    };
    ledger.keep_print(&learn::fingerprint([to, cc], subject), at)?;
    ledger.hear(Some(account), &learn::sent(&from, [to, cc, bcc], at))
}
