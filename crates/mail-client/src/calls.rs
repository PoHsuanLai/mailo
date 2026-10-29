//! What the client does in `mail-core` for a caller that has no store: the same calls, with the
//! store the client's.
//!
//! Each handle ([`Client::compose`], [`Client::template`], …) is named for the `mail-core` module
//! it calls, and each of its methods for the function, taking what that function takes but the
//! store. They run on the caller's thread, as a small read or write the caller needs the answer to
//! at once.

mod act {
    use crate::Client;
    use mail_core::undo::Undo;
    use mail_domain::*;

    /// What the client calls in `mail_core::act`.
    #[derive(Clone, Copy, Debug)]
    pub struct Act<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::act`, over this client's store.
        pub fn acting(&self) -> Act<'_> {
            Act(self)
        }
    }

    impl Act<'_> {
        /// `mail_core::act::perform`.
        pub fn perform(
            &self,
            thread: ThreadId,
            op: Op,
            clock: mail_core::when::Clock,
        ) -> Option<Undo> {
            mail_core::act::perform(self.0.store(), thread, op, clock)
        }

        /// `mail_core::act::take_back`.
        pub fn take_back(&self, entry: &Undo) -> bool {
            mail_core::act::take_back(self.0.store(), entry)
        }
    }
}
pub use act::Act;

mod attach {
    use crate::Client;
    use mail_core::error::CoreError;
    use mail_domain::*;
    use std::path::{Path, PathBuf};

    /// What the client calls in `mail_core::attach`.
    #[derive(Clone, Copy, Debug)]
    pub struct Attach<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::attach`, over this client's store.
        pub fn attach(&self) -> Attach<'_> {
            Attach(self)
        }
    }

    impl Attach<'_> {
        /// `mail_core::attach::save`.
        pub fn save(
            &self,
            message: MessageId,
            index: usize,
            dir: &Path,
        ) -> Result<PathBuf, CoreError> {
            mail_core::attach::save(self.0.store(), message, index, dir)
        }
    }
}
pub use attach::Attach;

mod compose {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::compose::*;
    use mail_core::error::CoreError;
    use mail_domain::*;
    use porter_core::AccountId;

    /// What the client calls in `mail_core::compose`.
    #[derive(Clone, Copy, Debug)]
    pub struct Compose<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::compose`, over this client's store.
        pub fn compose(&self) -> Compose<'_> {
            Compose(self)
        }
    }

    impl Compose<'_> {
        /// `mail_core::compose::account_for`.
        pub fn account_for(&self, wanted: Option<&str>) -> Result<AccountId, CoreError> {
            mail_core::compose::account_for(self.0.store(), wanted)
        }

        /// `mail_core::compose::attach_bytes`.
        pub fn attach_bytes(
            &self,
            draft: DraftId,
            name: &str,
            bytes: &[u8],
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::attach_bytes(self.0.store(), draft, name, bytes, now)
        }

        /// `mail_core::compose::attached_to`.
        pub fn attached_to(&self, draft: &Draft) -> Vec<(String, String)> {
            mail_core::compose::attached_to(self.0.store(), draft)
        }

        /// `mail_core::compose::discard`.
        pub fn discard(&self, draft: DraftId) -> Result<String, CoreError> {
            mail_core::compose::discard(self.0.store(), draft)
        }

        /// `mail_core::compose::draft_forward`.
        pub fn draft_forward(
            &self,
            message: MessageId,
            to: &[Address],
            body: &str,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::draft_forward(self.0.store(), message, to, body, now)
        }

        /// `mail_core::compose::draft_forward_attached`.
        pub fn draft_forward_attached(
            &self,
            message: MessageId,
            to: &[Address],
            body: &str,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::draft_forward_attached(self.0.store(), message, to, body, now)
        }

        /// `mail_core::compose::draft_mailto`.
        pub fn draft_mailto(
            &self,
            account: AccountId,
            link: &mail_mime::MailtoUri,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::draft_mailto(self.0.store(), account, link, now)
        }

        /// `mail_core::compose::draft_new`.
        pub fn draft_new(
            &self,
            account: AccountId,
            to: &[Address],
            subject: &str,
            body: &str,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::draft_new(self.0.store(), account, to, subject, body, now)
        }

        /// `mail_core::compose::draft_reply`.
        pub fn draft_reply(
            &self,
            message: MessageId,
            scope: ReplyScope,
            body: &str,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::draft_reply(self.0.store(), message, scope, body, now)
        }

        /// `mail_core::compose::identity_of`.
        pub fn identity_of(
            &self,
            account: AccountId,
            wanted: Option<IdentityId>,
        ) -> Result<Identity, CoreError> {
            mail_core::compose::identity_of(self.0.store(), account, wanted)
        }

        /// `mail_core::compose::move_draft_to`.
        pub fn move_draft_to(
            &self,
            draft: DraftId,
            account: AccountId,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::move_draft_to(self.0.store(), draft, account, now)
        }

        /// `mail_core::compose::queue_with`.
        pub fn queue_with(
            &self,
            secrets: &dyn mail_runtime::SigningStore,
            ask: mail_core::pgp::Ask<'_>,
            draft: DraftId,
            leaves: Leaves,
            now: DateTime<Utc>,
        ) -> Result<(Draft, mail_mime::Posting), SendError> {
            mail_core::compose::queue_with(self.0.store(), secrets, ask, draft, leaves, now)
        }

        /// `mail_core::compose::save`.
        pub fn save(&self, draft: &Draft) -> Result<(), CoreError> {
            mail_core::compose::save(self.0.store(), draft)
        }

        /// `mail_core::compose::sending_accounts`.
        pub fn sending_accounts(&self) -> Vec<(String, AccountId)> {
            mail_core::compose::sending_accounts(self.0.store())
        }

        /// `mail_core::compose::attach_inline`.
        pub fn attach_inline(
            &self,
            draft: DraftId,
            name: &str,
            bytes: &[u8],
            now: DateTime<Utc>,
        ) -> Result<(Draft, String), CoreError> {
            mail_core::compose::attach_inline(self.0.store(), draft, name, bytes, now)
        }

        /// `mail_core::compose::copy_myself`.
        pub fn copy_myself(&self, draft: DraftId, to: Myself) -> Result<Draft, CoreError> {
            mail_core::compose::copy_myself(self.0.store(), draft, to)
        }

        /// `mail_core::compose::draft_again`.
        pub fn draft_again(
            &self,
            message: MessageId,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::draft_again(self.0.store(), message, now)
        }

        /// `mail_core::compose::draft_answer`.
        pub fn draft_answer(
            &self,
            message: MessageId,
            to: Answer,
            quote: Quote<'_>,
            now: DateTime<Utc>,
            placement: mail_core::signature::Placement,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::draft_answer(self.0.store(), message, to, quote, now, placement)
        }

        /// `mail_core::compose::inline_parts`.
        pub fn inline_parts(&self, draft: &Draft) -> Vec<mail_mime::ParsedPart> {
            mail_core::compose::inline_parts(self.0.store(), draft)
        }

        /// `mail_core::compose::keep_on_server`.
        pub fn keep_on_server(
            &self,
            draft: DraftId,
            now: DateTime<Utc>,
        ) -> Result<bool, CoreError> {
            mail_core::compose::keep_on_server(self.0.store(), draft, now)
        }

        /// `mail_core::compose::move_draft_to_identity`.
        pub fn move_draft_to_identity(
            &self,
            draft: DraftId,
            identity: IdentityId,
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::compose::move_draft_to_identity(self.0.store(), draft, identity, now)
        }

        /// `mail_core::compose::queue_sized`.
        pub fn queue_sized(
            &self,
            secrets: &dyn mail_runtime::SigningStore,
            ask: mail_core::pgp::Ask<'_>,
            draft: DraftId,
            leaves: Leaves,
            pictures: PictureSize,
            now: DateTime<Utc>,
        ) -> Result<(Draft, mail_mime::Posting), SendError> {
            mail_core::compose::queue_sized(
                self.0.store(),
                secrets,
                ask,
                draft,
                leaves,
                pictures,
                now,
            )
        }

        /// `mail_core::compose::withdraw_redirect`.
        pub fn withdraw_redirect(&self, redirected: &Redirected) -> Result<(), CoreError> {
            mail_core::compose::withdraw_redirect(self.0.store(), redirected)
        }

        /// `mail_core::compose::written_as`.
        pub fn written_as(&self, message: MessageId) -> Option<Written> {
            mail_core::compose::written_as(self.0.store(), message)
        }

        /// `mail_core::compose::unsend`.
        pub fn unsend(&self, draft: DraftId, now: DateTime<Utc>) -> Result<Draft, CoreError> {
            mail_core::compose::unsend(self.0.store(), draft, now)
        }
    }
}
pub use compose::Compose;

mod contacts {
    use crate::Client;
    use mail_core::contacts::*;
    use mail_core::error::CoreError;

    /// What the client calls in `mail_core::contacts`.
    #[derive(Clone, Copy, Debug)]
    pub struct Contacts<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::contacts`, over this client's store.
        pub fn book(&self) -> Contacts<'_> {
            Contacts(self)
        }
    }

    impl Contacts<'_> {
        /// `mail_core::contacts::export`.
        pub fn export(&self) -> Result<String, CoreError> {
            mail_core::contacts::export(self.0.store())
        }

        /// `mail_core::contacts::import`.
        pub fn import(&self, bytes: &[u8]) -> Result<Imported, CoreError> {
            mail_core::contacts::import(self.0.store(), bytes)
        }
    }
}
pub use contacts::Contacts;

mod contacts_groups {
    use crate::Client;
    use mail_core::contacts::groups::*;
    use mail_core::error::CoreError;
    use mail_store::{Group, GroupId};

    /// What the client calls in `mail_core::contacts::groups`.
    #[derive(Clone, Copy, Debug)]
    pub struct ContactsGroups<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::contacts::groups`, over this client's store.
        pub fn contacts_groups(&self) -> ContactsGroups<'_> {
            ContactsGroups(self)
        }
    }

    impl ContactsGroups<'_> {
        /// `mail_core::contacts::groups::offers`.
        pub fn offers(&self, typed: &str) -> Vec<Offer> {
            mail_core::contacts::groups::offers(self.0.store(), typed)
        }

        /// `mail_core::contacts::groups::add`.
        pub fn add(&self, id: &GroupId, typed: &str) -> Result<Group, GroupError> {
            mail_core::contacts::groups::add(self.0.store(), id, typed)
        }

        /// `mail_core::contacts::groups::create`.
        pub fn create(&self, name: &str) -> Result<Group, GroupError> {
            mail_core::contacts::groups::create(self.0.store(), name)
        }

        /// `mail_core::contacts::groups::forget`.
        pub fn forget(&self, id: &GroupId) -> Result<(), GroupError> {
            mail_core::contacts::groups::forget(self.0.store(), id)
        }

        /// `mail_core::contacts::groups::listed`.
        pub fn listed(&self, filter: &str) -> Result<Vec<Group>, CoreError> {
            mail_core::contacts::groups::listed(self.0.store(), filter)
        }

        /// `mail_core::contacts::groups::members`.
        pub fn members(&self, group: &Group) -> Vec<(String, Labelled)> {
            mail_core::contacts::groups::members(self.0.store(), group)
        }

        /// `mail_core::contacts::groups::remove`.
        pub fn remove(&self, id: &GroupId, uri: &str) -> Result<Group, GroupError> {
            mail_core::contacts::groups::remove(self.0.store(), id, uri)
        }

        /// `mail_core::contacts::groups::rename`.
        pub fn rename(&self, id: &GroupId, name: &str) -> Result<Group, GroupError> {
            mail_core::contacts::groups::rename(self.0.store(), id, name)
        }
    }
}
pub use contacts_groups::ContactsGroups;

mod contacts_people {
    use crate::Client;
    use mail_core::contacts::people::{Added, Details, PersonError};
    use mail_core::error::CoreError;
    use mail_store::{CardId, PersonCard};

    /// What the client calls in `mail_core::contacts::people`.
    #[derive(Clone, Copy, Debug)]
    pub struct ContactsPeople<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::contacts::people`, over this client's store.
        pub fn people(&self) -> ContactsPeople<'_> {
            ContactsPeople(self)
        }
    }

    impl ContactsPeople<'_> {
        /// `mail_core::contacts::people::add_sender`.
        pub fn add_sender(&self, address: &str, name: &str) -> Result<Added, PersonError> {
            mail_core::contacts::people::add_sender(self.0.store(), address, name)
        }

        /// `mail_core::contacts::people::create`.
        pub fn create(&self, details: &Details) -> Result<PersonCard, PersonError> {
            mail_core::contacts::people::create(self.0.store(), details)
        }

        /// `mail_core::contacts::people::forget`.
        pub fn forget(&self, id: &CardId) -> Result<(), PersonError> {
            mail_core::contacts::people::forget(self.0.store(), id)
        }

        /// `mail_core::contacts::people::listed`.
        pub fn listed(&self, filter: &str) -> Result<Vec<PersonCard>, CoreError> {
            mail_core::contacts::people::listed(self.0.store(), filter)
        }

        /// `mail_core::contacts::people::photo_of`.
        pub fn photo_of(&self, address: &str) -> Option<String> {
            mail_core::contacts::people::photo_of(self.0.store(), address)
        }

        /// `mail_core::contacts::people::save`.
        pub fn save(&self, id: &CardId, details: &Details) -> Result<PersonCard, PersonError> {
            mail_core::contacts::people::save(self.0.store(), id, details)
        }
    }
}
pub use contacts_people::ContactsPeople;

mod export {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::environment::Environment;
    use mail_core::error::CoreError;
    use mail_core::export::Target;
    use mail_core::export::*;
    use mail_domain::*;
    use std::path::{Path, PathBuf};

    /// What the client calls in `mail_core::export`.
    #[derive(Clone, Copy, Debug)]
    pub struct Export<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::export`, over this client's store.
        pub fn export(&self) -> Export<'_> {
            Export(self)
        }
    }

    impl Export<'_> {
        /// `mail_core::export::export`.
        pub fn export(
            &self,
            env: &Environment,
            messages: &[MessageId],
            target: &Target,
            now: DateTime<Utc>,
            progress: &mut dyn FnMut(&Exported),
        ) -> Result<Exported, CoreError> {
            mail_core::export::export(self.0.store(), env, messages, target, now, progress)
        }

        /// `mail_core::export::raw_head`.
        pub fn raw_head(
            &self,
            id: MessageId,
            limit: usize,
        ) -> Result<Option<mail_core::export::RawHead>, CoreError> {
            mail_core::export::raw_head(self.0.store(), id, limit)
        }

        /// `mail_core::export::save_eml`.
        pub fn save_eml(&self, id: MessageId, dir: &Path) -> Result<Option<PathBuf>, CoreError> {
            mail_core::export::save_eml(self.0.store(), id, dir)
        }

        /// `mail_core::export::select`.
        pub fn select(&self, query: &str, now: DateTime<Utc>) -> Result<Vec<MessageId>, CoreError> {
            mail_core::export::select(self.0.store(), query, now)
        }
    }
}
pub use export::Export;

mod folder {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::folder::*;
    use mail_domain::*;
    use porter_core::AccountId;

    /// What the client calls in `mail_core::folder`.
    #[derive(Clone, Copy, Debug)]
    pub struct Folder<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::folder`, over this client's store.
        pub fn folder(&self) -> Folder<'_> {
            Folder(self)
        }
    }

    impl Folder<'_> {
        /// `mail_core::folder::change`.
        pub fn change(
            &self,
            account: AccountId,
            work: FolderWork,
            now: DateTime<Utc>,
        ) -> Result<Applied, Refusal> {
            mail_core::folder::change(self.0.store(), account, work, now)
        }
    }
}
pub use folder::Folder;

mod follow_up {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::error::CoreError;
    use mail_core::follow_up::*;
    use mail_core::notify::Notifier;
    use mail_domain::*;

    /// What the client calls in `mail_core::follow_up`.
    #[derive(Clone, Copy, Debug)]
    pub struct FollowUp<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::follow_up`, over this client's store.
        pub fn follow_up(&self) -> FollowUp<'_> {
            FollowUp(self)
        }
    }

    impl FollowUp<'_> {
        /// `mail_core::follow_up::after_queue`.
        pub fn after_queue(
            &self,
            draft: &Draft,
            raw: &[u8],
            at: Option<DateTime<Utc>>,
            set: DateTime<Utc>,
        ) -> Result<(), CoreError> {
            mail_core::follow_up::after_queue(self.0.store(), draft, raw, at, set)
        }

        /// `mail_core::follow_up::next_due`.
        pub fn next_due(&self) -> Option<DateTime<Utc>> {
            mail_core::follow_up::next_due(self.0.store())
        }

        /// `mail_core::follow_up::sweep_and_announce`.
        pub fn sweep_and_announce(
            &self,
            notifier: Option<&dyn Notifier>,
            now: DateTime<Utc>,
        ) -> Result<Swept, CoreError> {
            mail_core::follow_up::sweep_and_announce(self.0.store(), notifier, now)
        }
    }
}
pub use follow_up::FollowUp;

mod history {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_domain::*;

    /// What the client calls in `mail_core::history`.
    #[derive(Clone, Copy, Debug)]
    pub struct History<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::history`, over this client's store.
        pub fn history(&self) -> History<'_> {
            History(self)
        }
    }

    impl History<'_> {
        /// `mail_core::history::record`.
        pub fn record(&self, thread: ThreadId, now: DateTime<Utc>) {
            mail_core::history::record(self.0.store(), thread, now)
        }
    }
}
pub use history::History;

mod invite {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::Store as _;
    use mail_core::error::CoreError;
    use mail_core::invite::*;
    use mail_domain::*;

    /// What the client calls in `mail_core::invite`.
    #[derive(Clone, Copy, Debug)]
    pub struct Invite<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::invite`, over this client's store.
        pub fn invite(&self) -> Invite<'_> {
            Invite(self)
        }
    }

    impl Invite<'_> {
        /// `mail_core::invite::answer`.
        pub fn answer(
            &self,
            message: MessageId,
            attendance: Attendance,
            comment: Option<&str>,
            now: DateTime<Utc>,
        ) -> Result<Answered, CoreError> {
            mail_core::invite::answer(self.0.store(), message, attendance, comment, now)
        }

        /// `mail_core::invite::export`.
        pub fn export(&self, message: MessageId) -> Result<Vec<u8>, CoreError> {
            mail_core::invite::export(self.0.store(), message)
        }

        /// The event of `message`'s invitation as Add to Calendar puts it in a calendar: with the
        /// answer the reader gave it, when that answer is to this version of it. Times are
        /// written in `zone`.
        pub fn event_to_add<Z: chrono::TimeZone>(
            &self,
            message: MessageId,
            zone: &Z,
        ) -> Result<porter_calendar::Event, CoreError> {
            let store = self.0.store();
            let answered = match state(store, &store.message(message)?)? {
                InviteState::Shown { invite, answered } => answered
                    .filter(|answer| answer.sequence >= invite.sequence)
                    .map(|answer| answer.attendance),
                _ => None,
            };
            event_of(store, message, answered, zone)
        }

        /// `mail_core::invite::event_of`, as `message`'s invitation reads once `attendance` is
        /// the answer.
        pub fn event_answered<Z: chrono::TimeZone>(
            &self,
            message: MessageId,
            attendance: Attendance,
            zone: &Z,
        ) -> Result<porter_calendar::Event, CoreError> {
            event_of(self.0.store(), message, Some(attendance), zone)
        }

        /// The `UID` of the event `message` cancels: `NoInvitation` when it cancels none.
        pub fn cancelled_uid(&self, message: MessageId) -> Result<String, CoreError> {
            let store = self.0.store();
            match state(store, &store.message(message)?)? {
                InviteState::Shown { invite, .. } if invite.kind == mail_pim::Kind::Cancelled => {
                    Ok(invite.uid)
                }
                _ => Err(CoreError::NoInvitation),
            }
        }

        /// `mail_core::invite::propose`.
        pub fn propose(
            &self,
            message: MessageId,
            start: DateTime<Utc>,
            comment: Option<&str>,
            now: DateTime<Utc>,
        ) -> Result<Proposed, CoreError> {
            mail_core::invite::propose(self.0.store(), message, start, comment, now)
        }
    }
}
pub use invite::Invite;

mod pgp {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::pgp::PgpError;
    use mail_core::protect::Seal;
    use mail_domain::*;

    /// What the client calls in `mail_core::pgp`.
    #[derive(Clone, Copy, Debug)]
    pub struct Pgp<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::pgp`, over this client's store.
        pub fn pgp(&self) -> Pgp<'_> {
            Pgp(self)
        }
    }

    impl Pgp<'_> {
        /// `mail_core::pgp::OpenPgp::check`.
        pub fn check(
            &self,
            draft: &Draft,
            identity: &Identity,
            now: DateTime<Utc>,
        ) -> Result<(), PgpError> {
            mail_core::pgp::OpenPgp::check(self.0.store(), draft, identity, now)
        }

        /// `mail_core::pgp::own_key`.
        pub fn own_key(&self, address: &str) -> Result<Option<PgpKey>, PgpError> {
            mail_core::pgp::own_key(self.0.store(), address)
        }
    }
}
pub use pgp::Pgp;

mod pgp_keys {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::pgp::keys::*;
    use mail_core::pgp::{PgpError, WithSecret};
    use mail_domain::*;
    use mail_runtime::SigningStore;

    /// What the client calls in `mail_core::pgp::keys`.
    #[derive(Clone, Copy, Debug)]
    pub struct PgpKeys<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::pgp::keys`, over this client's store.
        pub fn keys(&self) -> PgpKeys<'_> {
            PgpKeys(self)
        }
    }

    impl PgpKeys<'_> {
        /// `mail_core::pgp::keys::delete`.
        pub fn delete(
            &self,
            secrets: &dyn SigningStore,
            key: &PgpKey,
            with_secret: WithSecret,
        ) -> Result<(), PgpError> {
            mail_core::pgp::keys::delete(self.0.store(), secrets, key, with_secret)
        }

        /// `mail_core::pgp::keys::export_secret`.
        pub fn export_secret(
            &self,
            secrets: &dyn SigningStore,
            key: &PgpKey,
        ) -> Result<String, PgpError> {
            mail_core::pgp::keys::export_secret(self.0.store(), secrets, key)
        }

        /// `mail_core::pgp::keys::generate`.
        pub fn generate(
            &self,
            secrets: &dyn SigningStore,
            address: &str,
            now: DateTime<Utc>,
        ) -> Result<PgpKey, PgpError> {
            mail_core::pgp::keys::generate(self.0.store(), secrets, address, now)
        }

        /// `mail_core::pgp::keys::import`.
        pub fn import(
            &self,
            secrets: &dyn SigningStore,
            bytes: &[u8],
            now: DateTime<Utc>,
        ) -> Result<Vec<Imported>, PgpError> {
            mail_core::pgp::keys::import(self.0.store(), secrets, bytes, now)
        }

        /// `mail_core::pgp::keys::restore`.
        pub fn restore(&self, key: &PgpKey) -> Result<(), PgpError> {
            mail_core::pgp::keys::restore(self.0.store(), key)
        }

        /// `mail_core::pgp::keys::verify`.
        pub fn verify(&self, fingerprint: Fingerprint) -> Result<(), PgpError> {
            mail_core::pgp::keys::verify(self.0.store(), fingerprint)
        }
    }
}
pub use pgp_keys::PgpKeys;

mod preview {
    use crate::Client;
    use mail_core::attachment_image::*;
    use mail_domain::*;

    /// What the client calls in `mail_core::attachment_image`.
    #[derive(Clone, Copy, Debug)]
    pub struct Preview<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::attachment_image`, over this client's store.
        pub fn preview(&self) -> Preview<'_> {
            Preview(self)
        }
    }

    impl Preview<'_> {
        /// `mail_core::attachment_image::load`.
        pub fn load(
            &self,
            message: MessageId,
            index: usize,
        ) -> Result<(Kind, Vec<u8>), Unavailable> {
            mail_core::attachment_image::load(self.0.store(), message, index)
        }
    }
}
pub use preview::Preview;

mod print {
    use crate::Client;
    use chrono::{DateTime, TimeZone, Utc};
    use mail_core::error::CoreError;
    use mail_core::html_document::*;
    use mail_core::when::Clock;
    use mail_mime::print::{Options, Pages};

    /// What the client calls in `mail_core::html_document`.
    #[derive(Clone, Copy, Debug)]
    pub struct Print<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::html_document`, over this client's store.
        pub fn print(&self) -> Print<'_> {
            Print(self)
        }
    }

    impl Print<'_> {
        /// `mail_core::html_document::document`.
        pub fn document<Tz>(
            &self,
            id: uuid::Uuid,
            zone: &Tz,
            now: DateTime<Utc>,
            pages: Pages,
            clock: Clock,
        ) -> Result<Printed, CoreError>
        where
            Tz: TimeZone,
            Tz::Offset: std::fmt::Display,
        {
            mail_core::html_document::document(self.0.store(), id, zone, now, pages, clock)
        }

        /// `mail_core::html_document::document_with`.
        pub fn document_with<Tz>(
            &self,
            id: uuid::Uuid,
            zone: &Tz,
            now: DateTime<Utc>,
            options: &Options<'_>,
            pictures: Option<&Pictures<'_>>,
        ) -> Result<Printed, CoreError>
        where
            Tz: TimeZone,
            Tz::Offset: std::fmt::Display,
        {
            mail_core::html_document::document_with(
                self.0.store(),
                id,
                zone,
                now,
                options,
                pictures,
            )
        }
    }
}
pub use print::Print;

mod query {
    use crate::Client;
    use mail_domain::*;

    /// What the client calls in `mail_core::query`.
    #[derive(Clone, Copy, Debug)]
    pub struct Query<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::query`, over this client's store.
        pub fn query(&self) -> Query<'_> {
            Query(self)
        }
    }

    impl Query<'_> {
        /// `mail_core::query::known_labels`.
        pub fn known_labels(&self) -> Vec<(String, LabelId)> {
            mail_core::query::known_labels(self.0.store())
        }

        /// `mail_core::vip::read`.
        pub fn vips(&self) -> mail_core::vip::Vips {
            mail_core::vip::read(self.0.store())
        }

        /// `mail_core::vip::listed`.
        pub fn listed_vips(&self) -> Result<Vec<mail_core::vip::Vip>, mail_core::error::CoreError> {
            mail_core::vip::listed(self.0.store())
        }

        /// `mail_core::vip::set`.
        pub fn set_vip(
            &self,
            address: &str,
            name: Option<&str>,
            mark: mail_core::vip::Mark,
        ) -> Result<(), mail_core::error::CoreError> {
            mail_core::vip::set(self.0.store(), address, name, mark)
        }

        /// `mail_core::query::known_accounts`.
        pub fn known_accounts(&self) -> Vec<(String, porter_core::AccountId)> {
            mail_core::query::known_accounts(self.0.store())
        }
    }
}
pub use query::Query;

mod receipt {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::error::CoreError;
    use mail_core::receipt::*;
    use mail_domain::*;

    /// What the client calls in `mail_core::receipt`.
    #[derive(Clone, Copy, Debug)]
    pub struct Receipt<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::receipt`, over this client's store.
        pub fn receipt(&self) -> Receipt<'_> {
            Receipt(self)
        }
    }

    impl Receipt<'_> {
        /// `mail_core::receipt::answer`.
        pub fn answer(
            &self,
            message: MessageId,
            answer: ReceiptAnswer,
            now: DateTime<Utc>,
        ) -> Result<Settled, CoreError> {
            mail_core::receipt::answer(self.0.store(), message, answer, now)
        }
    }
}
pub use receipt::Receipt;

mod rules_manage {
    use crate::Client;
    use chrono::{DateTime, TimeZone, Utc};
    use mail_core::error::CoreError;
    use mail_core::rules::ListedRule;
    use mail_core::rules::manage::Draft;
    use mail_core::rules::manage::*;
    use mail_domain::*;
    use porter_core::AccountId;

    /// What the client calls in `mail_core::rules::manage`.
    #[derive(Clone, Copy, Debug)]
    pub struct RulesManage<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::rules::manage`, over this client's store.
        pub fn rules_manage(&self) -> RulesManage<'_> {
            RulesManage(self)
        }
    }

    impl RulesManage<'_> {
        /// `mail_core::rules::manage::conversations`.
        pub fn conversations(&self, account: AccountId, now: DateTime<Utc>) -> usize {
            mail_core::rules::manage::conversations(self.0.store(), account, now)
        }

        /// `mail_core::rules::manage::delete`.
        pub fn delete(&self, rule: &Rule) -> Result<(), CoreError> {
            mail_core::rules::manage::delete(self.0.store(), rule)
        }

        /// `mail_core::rules::manage::labels`.
        pub fn labels(&self, account: AccountId) -> Vec<(String, LabelId)> {
            mail_core::rules::manage::labels(self.0.store(), account)
        }

        /// `mail_core::rules::manage::listed`.
        pub fn listed(&self, account: AccountId) -> Result<Vec<ListedRule>, CoreError> {
            mail_core::rules::manage::listed(self.0.store(), account)
        }

        /// `mail_core::rules::manage::matching`.
        pub fn matching(
            &self,
            account: AccountId,
            filter: Filter,
            now: DateTime<Utc>,
        ) -> Result<u64, CoreError> {
            mail_core::rules::manage::matching(self.0.store(), account, filter, now)
        }

        /// `mail_core::rules::manage::reorder`.
        pub fn reorder(&self, account: AccountId, id: RuleId, step: Step) -> Result<(), CoreError> {
            mail_core::rules::manage::reorder(self.0.store(), account, id, step)
        }

        /// `mail_core::rules::manage::restore`.
        pub fn restore(&self, rule: &Rule) -> Result<(), CoreError> {
            mail_core::rules::manage::restore(self.0.store(), rule)
        }

        /// `mail_core::rules::manage::run`.
        pub fn run(
            &self,
            rule: &Rule,
            now: DateTime<Utc>,
            report: &(dyn Fn(usize) + Sync),
        ) -> Result<mail_store::rules::Ran, CoreError> {
            mail_core::rules::manage::run(self.0.store(), rule, now, report)
        }

        /// `mail_core::rules::manage::save`.
        pub fn save<Tz: TimeZone>(
            &self,
            account: AccountId,
            draft: &Draft,
            zone: &Tz,
        ) -> Result<Rule, SaveError> {
            mail_core::rules::manage::save(self.0.store(), account, draft, zone)
        }

        /// `mail_core::rules::manage::switch`.
        pub fn switch(&self, rule: &Rule, state: RuleState) -> Result<(), CoreError> {
            mail_core::rules::manage::switch(self.0.store(), rule, state)
        }
    }
}
pub use rules_manage::RulesManage;

mod smime_certs {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::pgp::WithSecret;
    use mail_core::smime::SmimeError;
    use mail_core::smime::certs::*;
    use mail_domain::*;
    use mail_runtime::SigningStore;

    /// What the client calls in `mail_core::smime::certs`.
    #[derive(Clone, Copy, Debug)]
    pub struct SmimeCerts<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::smime::certs`, over this client's store.
        pub fn certs(&self) -> SmimeCerts<'_> {
            SmimeCerts(self)
        }
    }

    impl SmimeCerts<'_> {
        /// `mail_core::smime::certs::delete`.
        pub fn delete(
            &self,
            secrets: &dyn SigningStore,
            cert: &SmimeCert,
            with_secret: WithSecret,
        ) -> Result<(), SmimeError> {
            mail_core::smime::certs::delete(self.0.store(), secrets, cert, with_secret)
        }

        /// `mail_core::smime::certs::import`.
        pub fn import(
            &self,
            secrets: &dyn SigningStore,
            bytes: &[u8],
            password: &dyn Fn() -> Option<String>,
            now: DateTime<Utc>,
        ) -> Result<Vec<Imported>, SmimeError> {
            mail_core::smime::certs::import(self.0.store(), secrets, bytes, password, now)
        }

        /// `mail_core::smime::certs::restore`.
        pub fn restore(&self, cert: &SmimeCert) -> Result<(), SmimeError> {
            mail_core::smime::certs::restore(self.0.store(), cert)
        }

        /// `mail_core::smime::certs::trust`.
        pub fn trust(
            &self,
            fingerprint: CertFingerprint,
            trust: KeyTrust,
        ) -> Result<(), SmimeError> {
            mail_core::smime::certs::trust(self.0.store(), fingerprint, trust)
        }
    }
}
pub use smime_certs::SmimeCerts;

mod smime {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::protect::Seal;
    use mail_core::smime::SmimeError;
    use mail_domain::*;

    /// What the client calls in `mail_core::smime`.
    #[derive(Clone, Copy, Debug)]
    pub struct Smime<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::smime`, over this client's store.
        pub fn smime(&self) -> Smime<'_> {
            Smime(self)
        }
    }

    impl Smime<'_> {
        /// `mail_core::smime::Smime::check`.
        pub fn check(
            &self,
            draft: &Draft,
            identity: &Identity,
            now: DateTime<Utc>,
        ) -> Result<(), SmimeError> {
            mail_core::smime::Smime::check(self.0.store(), draft, identity, now)
        }

        /// `mail_core::smime::own_cert`.
        pub fn own_cert(
            &self,
            address: &str,
            now: DateTime<Utc>,
        ) -> Result<Option<SmimeCert>, SmimeError> {
            mail_core::smime::own_cert(self.0.store(), address, now)
        }
    }
}
pub use smime::Smime;

mod snooze {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::error::CoreError;
    use mail_domain::*;

    /// What the client calls in `mail_core::snooze`.
    #[derive(Clone, Copy, Debug)]
    pub struct Snooze<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::snooze`, over this client's store.
        pub fn snooze(&self) -> Snooze<'_> {
            Snooze(self)
        }
    }

    impl Snooze<'_> {
        /// `mail_core::snooze::pin`.
        pub fn pin(&self, thread: ThreadId, now: DateTime<Utc>) -> Result<bool, CoreError> {
            mail_core::snooze::pin(self.0.store(), thread, now)
        }
    }
}
pub use snooze::Snooze;

mod sync {
    use crate::Client;
    use porter_core::AccountId;

    /// What the client calls in `mail_core::sync`.
    #[derive(Clone, Copy, Debug)]
    pub struct Sync<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::sync`, over this client's store.
        pub fn sync(&self) -> Sync<'_> {
            Sync(self)
        }
    }

    impl Sync<'_> {
        /// `mail_core::sync::local_accounts`.
        pub fn local_accounts(&self) -> Vec<AccountId> {
            mail_core::sync::local_accounts(self.0.store())
        }
    }
}
pub use sync::Sync;

mod template {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::error::CoreError;
    use mail_domain::*;

    /// What the client calls in `mail_core::template`.
    #[derive(Clone, Copy, Debug)]
    pub struct Templates<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::template`, over this client's store.
        pub fn template(&self) -> Templates<'_> {
            Templates(self)
        }
    }

    impl Templates<'_> {
        /// `mail_core::template::all`.
        pub fn all(&self) -> Result<Vec<(String, Template)>, CoreError> {
            mail_core::template::all(self.0.store())
        }

        /// `mail_core::template::delete`.
        pub fn delete(&self, template: TemplateId) -> Result<String, CoreError> {
            mail_core::template::delete(self.0.store(), template)
        }

        /// `mail_core::template::save`.
        pub fn save(
            &self,
            draft: DraftId,
            name: &str,
            now: DateTime<Utc>,
        ) -> Result<Template, CoreError> {
            mail_core::template::save(self.0.store(), draft, name, now)
        }

        /// `mail_core::template::start`.
        pub fn start(
            &self,
            template: TemplateId,
            to: &[Address],
            now: DateTime<Utc>,
        ) -> Result<Draft, CoreError> {
            mail_core::template::start(self.0.store(), template, to, now)
        }
    }
}
pub use template::Templates;

mod transfer {
    use crate::Client;
    use chrono::{DateTime, Utc};
    use mail_core::error::CoreError;
    use mail_core::import::{Destination, Imported, Source};
    use mail_core::transfer::*;
    use porter_core::AccountId;

    /// What the client calls in `mail_core::transfer`.
    #[derive(Clone, Copy, Debug)]
    pub struct Transfer<'a>(pub(super) &'a Client);

    impl Client {
        /// `mail_core::transfer`, over this client's store.
        pub fn transfer(&self) -> Transfer<'_> {
            Transfer(self)
        }
    }

    impl Transfer<'_> {
        /// `mail_core::transfer::import_into`.
        pub fn import_into(
            &self,
            source: &Source,
            into: &Destination,
            now: DateTime<Utc>,
            progress: &mut dyn FnMut(&Imported),
        ) -> Result<(Imported, Option<AccountId>), CoreError> {
            mail_core::transfer::import_into(self.0.store(), source, into, now, progress)
        }

        /// `mail_core::transfer::selection_size`.
        pub fn selection_size(&self, query: &str, now: DateTime<Utc>) -> Result<usize, CoreError> {
            mail_core::transfer::selection_size(self.0.store(), query, now)
        }

        /// `mail_core::transfer::upload_folders`.
        pub fn upload_folders(&self) -> Vec<Upload> {
            mail_core::transfer::upload_folders(self.0.store())
        }
    }
}
pub use transfer::Transfer;

mod more;
pub use more::*;
