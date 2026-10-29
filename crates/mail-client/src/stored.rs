//! What is kept, read where the caller is: the store's own answers, for a caller that has no store.
//!
//! A small read the caller needs at once (the draft it is about to open, the key it is about to
//! show) and the few plain writes it makes by hand. What is drawn and must keep up with the store is
//! a query ([`crate::Query`]) instead, read off the caller's thread and read again as it moves.

use crate::Client;
use chrono::{DateTime, Utc};
use mail_domain::{
    CertFingerprint, Draft, DraftId, Filter, Fingerprint, Folder, InviteAnswer, Label, LabelId,
    Message, MessageId, Page, Patch, PgpKey, Placed, Query, ReceiptAnswer, Rule, SmimeCert, Thread,
    ThreadId, ThreadSummary, Vacation, View, ViewId,
};
use mail_store::{
    Blobs, Contact, Group, GroupId, Offline, Origin, OutboxEntry, SenderRecord, Store, StoreError,
    StoredAccount,
};
use porter_core::AccountId;

/// The store's answers, through a client ([`Client::stored`]).
///
/// Every call here may block: it runs on the caller's thread and waits for a reader or the
/// writer, behind whatever an ingest or a long query holds. So none is made from a frame being
/// drawn, nor from a handler on the thread that draws; what a frame shows is a query
/// ([`crate::Query`]) and what a press changes is an act ([`crate::Act`]).
#[derive(Clone, Copy, Debug)]
pub struct Stored<'a>(&'a Client);

impl Client {
    /// What is kept, read on the caller's thread. May block ([`Stored`]).
    pub fn stored(&self) -> Stored<'_> {
        Stored(self)
    }

    /// The bodies and attachments kept, by their blob.
    pub fn blobs(&self) -> Blobs<'_> {
        self.store().blobs()
    }

    /// What a search reads ([`mail_core::search::run`]).
    pub fn source(&self) -> &dyn mail_core::search::Source {
        self.store()
    }
}

impl Stored<'_> {
    pub fn identities(&self, account: AccountId) -> Result<Vec<mail_domain::Identity>, StoreError> {
        self.0.store().identities(account)
    }

    pub fn identity(
        &self,
        id: mail_domain::IdentityId,
    ) -> Result<Option<mail_domain::Identity>, StoreError> {
        self.0.store().identity(id)
    }

    pub fn account_prefs(
        &self,
        account: AccountId,
    ) -> Result<mail_store::AccountPrefs, StoreError> {
        self.0.store().account_prefs(account)
    }

    pub fn signature(
        &self,
        id: mail_domain::SignatureId,
    ) -> Result<mail_domain::Signature, StoreError> {
        self.0.store().signature(id)
    }

    pub fn followed_folders(&self) -> Result<Vec<mail_domain::MailboxRef>, StoreError> {
        self.0.store().followed_folders()
    }

    pub fn card(
        &self,
        id: &mail_store::CardId,
    ) -> Result<Option<mail_store::PersonCard>, StoreError> {
        self.0.store().card(id)
    }

    pub fn card_of(&self, address: &str) -> Result<Option<mail_store::PersonCard>, StoreError> {
        self.0.store().card_of(address)
    }

    pub fn cards(&self) -> Result<Vec<mail_store::PersonCard>, StoreError> {
        self.0.store().cards()
    }

    pub fn account(&self, id: AccountId) -> Result<Option<StoredAccount>, StoreError> {
        self.0.store().account(id)
    }

    pub fn list_accounts(&self) -> Result<Vec<StoredAccount>, StoreError> {
        self.0.store().list_accounts()
    }

    pub fn sender_history(&self) -> Result<Vec<SenderRecord>, StoreError> {
        self.0.store().sender_history()
    }

    /// What the account's server was last seen to support ([`mail_core::act::caps_here`]).
    pub fn caps(&self, account: AccountId, now: DateTime<Utc>) -> mail_domain::AccountCaps {
        mail_core::act::caps_here(self.0.store(), account, now)
    }

    /// How fresh the account is judged to be ([`mail_core::sync::fresh::Pace::of`]).
    pub fn pace(&self, account: AccountId) -> mail_core::sync::fresh::Pace {
        mail_core::sync::fresh::Pace::of(self.0.store(), account)
    }

    pub fn folders(&self, account: AccountId) -> Result<Vec<Folder>, StoreError> {
        self.0.store().folders(account)
    }

    /// Write the caller's own changes, atomically.
    pub fn apply(&self, account: AccountId, patch: &Patch) -> Result<(), StoreError> {
        self.0.store().apply(account, patch)
    }

    pub fn put_vacation(
        &self,
        account: AccountId,
        vacation: Option<&Vacation>,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.0.store().put_vacation(account, vacation, now)
    }

    pub fn put_view(&self, view: &View) -> Result<(), StoreError> {
        self.0.store().put_view(view)
    }

    pub fn thread(&self, id: ThreadId) -> Result<Thread, StoreError> {
        self.0.store().thread(id)
    }

    pub fn threads(
        &self,
        query: &Query,
        now: DateTime<Utc>,
    ) -> Result<Page<ThreadSummary>, StoreError> {
        self.0.store().threads(query, now)
    }

    pub fn count(&self, filter: &Filter, now: DateTime<Utc>) -> Result<u64, StoreError> {
        self.0.store().count(filter, now)
    }

    pub fn message(&self, id: MessageId) -> Result<Message, StoreError> {
        self.0.store().message(id)
    }

    pub fn placed(&self, message: MessageId) -> Result<Vec<Placed>, StoreError> {
        self.0.store().placed(message)
    }

    pub fn draft(&self, id: DraftId) -> Result<Draft, StoreError> {
        self.0.store().draft(id)
    }

    pub fn drafts(&self, account: AccountId) -> Result<Vec<Draft>, StoreError> {
        self.0.store().drafts(account)
    }

    pub fn outbox_due(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<OutboxEntry>, StoreError> {
        self.0.store().outbox_due(account, now)
    }

    pub fn offline(&self, account: AccountId) -> Result<Offline, StoreError> {
        self.0.store().offline(account)
    }

    pub fn set_account_description(
        &self,
        account: AccountId,
        description: Option<&str>,
    ) -> Result<(), StoreError> {
        self.0.store().set_account_description(account, description)
    }

    pub fn set_account_syncing(
        &self,
        account: AccountId,
        syncing: mail_store::Syncing,
    ) -> Result<(), StoreError> {
        self.0.store().set_account_syncing(account, syncing)
    }

    /// Keep `account`'s labels on its server as keywords, or here alone.
    pub fn set_account_keeps_labels(
        &self,
        account: AccountId,
        keeps: bool,
    ) -> Result<(), StoreError> {
        self.0
            .store()
            .set_account_keeps_labels(account, keeps, chrono::Utc::now())
    }

    pub fn set_folder_role(
        &self,
        account: AccountId,
        role: mail_domain::MailboxRole,
        path: Option<&str>,
    ) -> Result<(), StoreError> {
        self.0.store().set_folder_role(account, role, path)
    }

    pub fn vacation(&self, account: AccountId) -> Result<Option<Vacation>, StoreError> {
        self.0.store().vacation(account)
    }

    pub fn rules(&self, account: AccountId) -> Result<Vec<Rule>, StoreError> {
        self.0.store().rules(account)
    }

    pub fn labels(&self, account: AccountId) -> Result<Vec<Label>, StoreError> {
        self.0.store().labels(account)
    }

    pub fn create_label(&self, account: AccountId, name: &str) -> Result<LabelId, StoreError> {
        self.0.store().create_label(account, name)
    }

    pub fn delete_view(&self, id: ViewId) -> Result<(), StoreError> {
        self.0.store().delete_view(id)
    }

    pub fn contacts(&self) -> Result<Vec<Contact>, StoreError> {
        self.0.store().contacts()
    }

    pub fn contact(&self, address: &str) -> Result<Option<Contact>, StoreError> {
        self.0.store().contact(address)
    }

    pub fn contacts_matching(&self, typed: &str, k: usize) -> Result<Vec<Contact>, StoreError> {
        self.0.store().contacts_matching(typed, k)
    }

    pub fn put_contact(
        &self,
        address: &str,
        name: Option<&str>,
        origin: &Origin,
    ) -> Result<Contact, StoreError> {
        self.0.store().put_contact(address, name, origin)
    }

    pub fn groups(&self) -> Result<Vec<Group>, StoreError> {
        self.0.store().groups()
    }

    pub fn group(&self, id: &GroupId) -> Result<Option<Group>, StoreError> {
        self.0.store().group(id)
    }

    pub fn put_group(&self, group: &Group) -> Result<(), StoreError> {
        self.0.store().put_group(group)
    }

    pub fn pgp_keys(&self) -> Result<Vec<PgpKey>, StoreError> {
        self.0.store().pgp_keys()
    }

    pub fn pgp_key(&self, fingerprint: Fingerprint) -> Result<Option<PgpKey>, StoreError> {
        self.0.store().pgp_key(fingerprint)
    }

    pub fn smime_certs(&self) -> Result<Vec<SmimeCert>, StoreError> {
        self.0.store().smime_certs()
    }

    pub fn smime_cert(
        &self,
        fingerprint: CertFingerprint,
    ) -> Result<Option<SmimeCert>, StoreError> {
        self.0.store().smime_cert(fingerprint)
    }

    pub fn receipt_answer(&self, message: MessageId) -> Result<Option<ReceiptAnswer>, StoreError> {
        self.0.store().receipt_answer(message)
    }

    pub fn invite_answer(&self, message: MessageId) -> Result<Option<InviteAnswer>, StoreError> {
        self.0.store().invite_answer(message)
    }
}
