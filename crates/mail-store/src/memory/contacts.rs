//! The address book in memory. Kept in step with `sqlite/contacts.rs`, which the parity tests
//! in `tests/contacts.rs` hold it to.
//!
//! One difference, and it is about input rather than rules: this store holds no raw bytes, so
//! it cannot see a `List-Id` header and judges a sender by its address alone.

use super::Inner;
use crate::StoreError;
use crate::contact::learn::{self, Event, Needle, Row, Seen};
use crate::contact::{AddressBook, Contact, Kind, Origin};
use chrono::{DateTime, Utc};
use mail_domain::{AccountId, Address, DraftId, MailboxRole, Message, MessageId};

impl Inner {
    pub(super) fn learn_fetched(&mut self, account: AccountId, id: MessageId, message: &Message) {
        if matches!(message.mailbox, MailboxRole::Spam | MailboxRole::Drafts) {
            return;
        }
        if !self.counted.insert(id) {
            return;
        }
        let seen = Seen {
            mailbox: message.mailbox,
            date: message.date,
            from: &message.from,
            to: &message.to,
            cc: &message.cc,
            bcc: &message.bcc,
            subject: &message.subject,
            sender: Kind::Person,
        };
        let mut events = learn::events(&seen);
        if message.mailbox == MailboxRole::Sent {
            let print = learn::fingerprint([&message.to, &message.cc], &message.subject);
            if let Some(at) = self.sent_prints.iter().position(|p| *p == print) {
                self.sent_prints.remove(at);
                events.retain(|(_, e)| *e == Event::Own);
            }
        }
        self.hear(Some(account), &events);
    }

    pub(super) fn learn_submission(
        &mut self,
        account: AccountId,
        draft: DraftId,
        mail_from: &str,
        rcpt_to: &[String],
        at: DateTime<Utc>,
    ) {
        let envelope: Vec<Address> = rcpt_to
            .iter()
            .map(|email| Address {
                name: None,
                email: email.clone(),
            })
            .collect();
        let draft = self.drafts.get(&draft).cloned();
        let (to, cc, bcc, subject) = match &draft {
            Some(d) => (&d.to[..], &d.cc[..], &d.bcc[..], d.subject.as_str()),
            None => (&envelope[..], &[][..], &[][..], ""),
        };
        let from = Address {
            name: None,
            email: mail_from.to_owned(),
        };
        self.sent_prints.push(learn::fingerprint([to, cc], subject));
        let events = learn::sent(&from, [to, cc, bcc], at);
        self.hear(Some(account), &events);
    }

    fn hear(&mut self, account: Option<AccountId>, events: &[(String, Event)]) {
        for (address, event) in events {
            let row = self
                .contacts
                .remove(address)
                .unwrap_or_else(|| Row::new(address));
            self.contacts
                .insert(address.clone(), learn::apply(row, account, event));
        }
    }

    /// Every row, in the order both stores rank them.
    fn ranked(&self) -> Vec<&Row> {
        let mut rows: Vec<&Row> = self.contacts.values().collect();
        rows.sort_by(|a, b| learn::rank(a, b));
        rows
    }

    pub(super) fn contacts_like(&self, typed: &str, k: usize) -> Vec<Contact> {
        let needle = Needle::new(typed);
        self.ranked()
            .into_iter()
            .filter(|row| row.contact.offered() && needle.matches(&learn::keys(&row.contact)))
            .take(k)
            .map(|row| row.contact.clone())
            .collect()
    }

    pub(super) fn every_contact(&self) -> Vec<Contact> {
        self.ranked()
            .into_iter()
            .map(|r| r.contact.clone())
            .collect()
    }

    pub(super) fn one_contact(&self, address: &str) -> Option<Contact> {
        let address = learn::normalise(address)?;
        self.contacts.get(&address).map(|r| r.contact.clone())
    }

    pub(super) fn give_contact(
        &mut self,
        address: &str,
        name: Option<&str>,
        origin: &Origin,
    ) -> Result<Contact, StoreError> {
        let address =
            learn::normalise(address).ok_or_else(|| StoreError::BadAddress(address.to_owned()))?;
        let row = self
            .contacts
            .remove(&address)
            .unwrap_or_else(|| Row::new(&address));
        let row = learn::given(row, name, origin);
        let contact = row.contact.clone();
        self.contacts.insert(address, row);
        Ok(contact)
    }

    pub(super) fn drop_contact(&mut self, address: &str) -> bool {
        learn::normalise(address).is_some_and(|a| self.contacts.remove(&a).is_some())
    }

    pub(super) fn read_book(&self, url: &str) -> Option<AddressBook> {
        self.books.get(url).cloned()
    }

    pub(super) fn write_book(&mut self, book: &AddressBook) {
        self.books.insert(book.url.clone(), book.clone());
    }
}
