//! The address book in memory. Kept in step with `sqlite/contacts.rs`, which the parity tests
//! in `tests/store/contacts.rs` hold it to.
//!
//! One difference, and it is about input rather than rules: this store holds no raw bytes, so
//! the list-header check has none to read and judges a sender by its address alone.

use super::Inner;
use crate::StoreError;
use crate::contact::learn::{self, Event, Needle, Row};
use crate::contact::ledger::{self, Ledger};
use crate::contact::{AddressBook, Contact, Origin};
use chrono::{DateTime, Utc};
use mail_domain::{BlobId, DraftId, Message, MessageId};
use porter_core::AccountId;

impl Inner {
    pub(super) fn learn_fetched(&mut self, account: AccountId, id: MessageId, message: &Message) {
        // Nothing here fails: the trait's `Result` is for the SQLite store's rows.
        let _ = ledger::learn_fetched(self, account, id, message, None);
    }

    pub(super) fn learn_submission(
        &mut self,
        account: AccountId,
        draft: DraftId,
        mail_from: &str,
        rcpt_to: &[String],
        at: DateTime<Utc>,
    ) {
        let draft = self.drafts.get(&draft).cloned();
        // Learning who was written to is a hint: a send never fails over it.
        let _ = ledger::learn_sent(self, account, draft.as_ref(), mail_from, rcpt_to, at);
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

impl Ledger for Inner {
    fn first_count(&mut self, id: MessageId) -> Result<bool, StoreError> {
        Ok(self.counted.insert(id))
    }

    fn head(&self, _raw: BlobId) -> Option<Vec<u8>> {
        None
    }

    fn take_print(&mut self, print: &str) -> Result<bool, StoreError> {
        let Some(at) = self.sent_prints.iter().position(|p| p == print) else {
            return Ok(false);
        };
        self.sent_prints.remove(at);
        Ok(true)
    }

    fn keep_print(&mut self, print: &str, _at: DateTime<Utc>) -> Result<(), StoreError> {
        self.sent_prints.push(print.to_owned());
        Ok(())
    }

    fn hear(
        &mut self,
        account: Option<AccountId>,
        events: &[(String, Event)],
    ) -> Result<(), StoreError> {
        for (address, event) in events {
            let row = self
                .contacts
                .remove(address)
                .unwrap_or_else(|| Row::new(address));
            self.contacts
                .insert(address.clone(), learn::apply(row, account.clone(), event));
        }
        Ok(())
    }
}
