//! What a message is read into, kept so it is read once: its frame ([`Frames`]) and what else the
//! reader shows of it ([`Looks`]: the sender's checks, an invitation, a receipt asked for, a way
//! out of a list, what OpenPGP or S/MIME opened). The caches are the caller's, one for the app;
//! what fills them is read from the client's store.

use crate::Client;
use chrono::{DateTime, Utc};
use mail_core::SigningStore;
use mail_core::message::{
    Bodies, FrameBody, Frames, Invited, Looks, Offer, Opening, Receipt, Sent,
};
use mail_core::password::Password;
use mail_domain::{BlobId, Message, MessageId, ThreadId};
use mail_mime::{AuthResults, SanitizePolicy};
use std::sync::Arc;

impl Client {
    /// `looks`, filled from this client's store.
    pub fn looking<'a>(&'a self, looks: &'a Looks) -> Looking<'a> {
        Looking {
            client: self,
            of: looks,
        }
    }

    /// `frames`, filled from this client's store.
    pub fn framing<'a>(&'a self, frames: &'a Arc<Frames>) -> Framing<'a> {
        Framing {
            client: self,
            of: frames,
        }
    }
}

/// A [`Looks`] read through a client ([`Client::looking`]).
#[derive(Clone, Copy, Debug)]
pub struct Looking<'a> {
    client: &'a Client,
    of: &'a Looks,
}

impl Looking<'_> {
    /// [`Looks::checks`].
    pub fn checks(&self, message: MessageId, raw: BlobId) -> Option<AuthResults> {
        self.of.checks(self.client.store(), message, raw)
    }

    /// [`Looks::invite`].
    pub fn invite(&self, message: MessageId, body: Option<BlobId>) -> Option<Invited> {
        self.of.invite(self.client.store(), message, body)
    }

    /// [`Looks::invite_again`].
    pub fn invite_again(&self, message: MessageId) -> Option<Invited> {
        self.of.invite_again(self.client.store(), message)
    }

    /// [`Looks::receipts`].
    pub fn receipts(&self, key: &[(MessageId, Option<BlobId>)]) -> Vec<Receipt> {
        self.of.receipts(self.client.store(), key)
    }

    /// [`Looks::code`].
    pub fn code(&self, message: MessageId, body: Option<BlobId>) -> Option<String> {
        self.of.code(self.client.store(), message, body)
    }

    /// [`Looks::post`].
    pub fn post(&self, thread: ThreadId, key: &Bodies) -> Option<MessageId> {
        self.of.post(self.client.store(), thread, key)
    }

    /// [`Looks::receipt_again`].
    pub fn receipt_again(&self, message: MessageId) {
        self.of.receipt_again(self.client.store(), message)
    }

    /// [`Looks::offer`].
    pub fn offer(&self, thread: ThreadId, key: &Bodies) -> Option<Offer> {
        self.of.offer(self.client.store(), thread, key)
    }

    /// [`Looks::sealed`].
    pub fn sealed(
        &self,
        secrets: &dyn SigningStore,
        message: MessageId,
        body: Option<BlobId>,
        now: DateTime<Utc>,
    ) -> Opening {
        self.of
            .sealed(self.client.store(), secrets, message, body, now)
    }

    /// [`Looks::unlocked`].
    pub fn unlocked(
        &self,
        secrets: &dyn SigningStore,
        message: MessageId,
        body: Option<BlobId>,
        passphrase: Password,
        now: DateTime<Utc>,
    ) -> Opening {
        self.of
            .unlocked(self.client.store(), secrets, message, body, passphrase, now)
    }
}

/// [`Frames`] read through a client ([`Client::framing`]).
#[derive(Clone, Copy, Debug)]
pub struct Framing<'a> {
    client: &'a Client,
    of: &'a Arc<Frames>,
}

impl Framing<'_> {
    /// [`Frames::uncached`].
    pub fn uncached(&self, message: &Message, policy: SanitizePolicy) -> FrameBody {
        self.of.uncached(self.client.store(), message, policy)
    }

    /// [`Frames::rendered`].
    pub fn rendered(&self, message: &Message, policy: SanitizePolicy) -> Arc<FrameBody> {
        self.of.rendered(self.client.store(), message, policy)
    }

    /// [`Frames::render_inline`].
    pub fn render_inline(
        &self,
        message: &Message,
        policy: SanitizePolicy,
        room: &mut u64,
        sent: &Sent,
        later: &mut Vec<(Message, SanitizePolicy)>,
    ) -> Option<Arc<FrameBody>> {
        self.of
            .render_inline(self.client.store(), message, policy, room, sent, later)
    }

    /// [`Frames::warm`].
    pub fn warm(&self, threads: Vec<ThreadId>, policy: SanitizePolicy) {
        self.of.warm(self.client.store_arc(), threads, policy)
    }

    /// [`Frames::warm_now`].
    pub fn warm_now(
        &self,
        threads: &[ThreadId],
        policy: SanitizePolicy,
        still_wanted: impl Fn() -> bool,
    ) {
        self.of
            .warm_now(self.client.store(), threads, policy, still_wanted)
    }
}
