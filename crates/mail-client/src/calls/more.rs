//! Calls in `mail-core` the window makes that the first stages did not wrap: the same functions, with the store the client's.
// Generated from a list of function names; each is the `mail-core` function of the same name.

mod signature {
    #![allow(clippy::too_many_arguments)]
    use crate::Client;
    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::signature::*;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    /// What the client calls in `mail_core::signature`.
    #[derive(Clone, Copy, Debug)]
    pub struct SignatureCalls<'a>(pub(in super::super) &'a Client);

    impl Client {
        /// `mail_core::signature`, over this client's store.
        pub fn signing(&self) -> SignatureCalls<'_> {
            SignatureCalls(self)
        }
    }

    impl SignatureCalls<'_> {
        /// `mail_core::signature::of_identity`.
        pub fn of_identity(&self, identity: IdentityId) -> Result<Vec<Signature>, CoreError> {
            mail_core::signature::of_identity(self.0.store(), identity)
        }

        /// `mail_core::signature::default_for`.
        pub fn default_for(
            &self,
            identity: IdentityId,
            purpose: SignatureUse,
        ) -> Result<Option<Signature>, CoreError> {
            mail_core::signature::default_for(self.0.store(), identity, purpose)
        }

        /// `mail_core::signature::restore`.
        pub fn restore(&self, had: &Signature) -> Result<(), CoreError> {
            mail_core::signature::restore(self.0.store(), had)
        }
    }
}
pub use signature::SignatureCalls;

mod identity {
    #![allow(clippy::too_many_arguments)]
    use crate::Client;
    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::identity::*;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    /// What the client calls in `mail_core::identity`.
    #[derive(Clone, Copy, Debug)]
    pub struct IdentityCalls<'a>(pub(in super::super) &'a Client);

    impl Client {
        /// `mail_core::identity`, over this client's store.
        pub fn identities(&self) -> IdentityCalls<'_> {
            IdentityCalls(self)
        }
    }

    impl IdentityCalls<'_> {
        /// `mail_core::identity::of_account`.
        pub fn of_account(&self, account: AccountId) -> Result<Vec<Identity>, CoreError> {
            mail_core::identity::of_account(self.0.store(), account)
        }

        /// `mail_core::identity::set_name`.
        pub fn set_name(&self, account: AccountId, typed: &str) -> Result<Identity, CoreError> {
            mail_core::identity::set_name(self.0.store(), account, typed)
        }

        /// `mail_core::identity::add_alias`.
        pub fn add_alias(&self, account: AccountId, typed: &Typed) -> Result<Identity, CoreError> {
            mail_core::identity::add_alias(self.0.store(), account, typed)
        }

        /// `mail_core::identity::edit`.
        pub fn edit(&self, id: IdentityId, typed: &Typed) -> Result<Identity, CoreError> {
            mail_core::identity::edit(self.0.store(), id, typed)
        }
    }
}
pub use identity::IdentityCalls;

mod rules_block {
    #![allow(clippy::too_many_arguments)]
    use crate::Client;
    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::rules::block::*;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    /// What the client calls in `mail_core::rules::block`.
    #[derive(Clone, Copy, Debug)]
    pub struct RulesBlockCalls<'a>(pub(in super::super) &'a Client);

    impl Client {
        /// `mail_core::rules::block`, over this client's store.
        pub fn blocking(&self) -> RulesBlockCalls<'_> {
            RulesBlockCalls(self)
        }
    }

    impl RulesBlockCalls<'_> {
        /// `mail_core::rules::block::who_to_block`.
        pub fn who_to_block(&self, thread: ThreadId) -> Result<Option<String>, CoreError> {
            mail_core::rules::block::who_to_block(self.0.store(), thread)
        }

        /// `mail_core::rules::block::block`.
        pub fn block(&self, account: AccountId, email: &str) -> Result<Blocked, CoreError> {
            mail_core::rules::block::block(self.0.store(), account, email)
        }
    }
}
pub use rules_block::RulesBlockCalls;

mod erase {
    #![allow(clippy::too_many_arguments)]
    use crate::Client;
    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::erase::*;
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    /// What the client calls in `mail_core::erase`.
    #[derive(Clone, Copy, Debug)]
    pub struct EraseCalls<'a>(pub(in super::super) &'a Client);

    impl Client {
        /// `mail_core::erase`, over this client's store.
        pub fn erasing(&self) -> EraseCalls<'_> {
            EraseCalls(self)
        }
    }

    impl EraseCalls<'_> {
        /// `mail_core::erase::sweep`.
        pub fn sweep(&self, policy: &Policy, now: DateTime<Utc>) -> Vec<MessageId> {
            mail_core::erase::sweep(self.0.store(), policy, now)
        }
    }
}
pub use erase::EraseCalls;

mod export {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::export::*;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::super::export::Export<'_> {
        /// `mail_core::export::raw_message`.
        pub fn raw_message(&self, id: MessageId) -> Result<Option<Vec<u8>>, CoreError> {
            mail_core::export::raw_message(self.0.store(), id)
        }
    }
}
mod signature_more {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::signature::*;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::signature::SignatureCalls<'_> {
        /// `mail_core::signature::every`.
        pub fn every(&self) -> Result<Vec<(Identity, Vec<Signature>)>, CoreError> {
            mail_core::signature::every(self.0.store())
        }

        /// `mail_core::signature::create`.
        pub fn create(&self, identity: IdentityId, name: &str) -> Result<Signature, CoreError> {
            mail_core::signature::create(self.0.store(), identity, name)
        }

        /// `mail_core::signature::write`.
        pub fn write(&self, id: SignatureId, doc: &Doc) -> Result<Signature, CoreError> {
            mail_core::signature::write(self.0.store(), id, doc)
        }

        /// `mail_core::signature::rename`.
        pub fn rename(&self, id: SignatureId, name: &str) -> Result<Signature, CoreError> {
            mail_core::signature::rename(self.0.store(), id, name)
        }

        /// `mail_core::signature::make_default`.
        pub fn make_default(
            &self,
            id: SignatureId,
            purpose: SignatureUse,
            default: IsDefault,
        ) -> Result<Signature, CoreError> {
            mail_core::signature::make_default(self.0.store(), id, purpose, default)
        }

        /// `mail_core::signature::delete`.
        pub fn delete(&self, id: SignatureId) -> Result<Signature, CoreError> {
            mail_core::signature::delete(self.0.store(), id)
        }
    }
}
mod identity_more {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::identity::*;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::identity::IdentityCalls<'_> {
        /// `mail_core::identity::remove_alias`.
        pub fn remove_alias(&self, id: IdentityId) -> Result<(), CoreError> {
            mail_core::identity::remove_alias(self.0.store(), id)
        }
    }
}
mod graph_push {
    #![allow(clippy::too_many_arguments)]
    use crate::Client;
    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_core::sync::graph_push::*;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    /// What the client calls in `mail_core::sync::graph_push`.
    #[derive(Clone, Copy, Debug)]
    pub struct GraphPushCalls<'a>(pub(in super::super) &'a Client);

    impl Client {
        /// `mail_core::sync::graph_push`, over this client's store.
        pub fn graph_pushing(&self) -> GraphPushCalls<'_> {
            GraphPushCalls(self)
        }
    }

    impl GraphPushCalls<'_> {
        /// `mail_core::sync::graph_push::relay`.
        pub fn relay(&self, account: AccountId) -> Option<String> {
            mail_core::sync::graph_push::relay(self.0.store(), account)
        }

        /// `mail_core::sync::graph_push::set_relay`.
        pub fn set_relay(
            &self,
            account: AccountId,
            text: &str,
        ) -> Result<Option<String>, CoreError> {
            mail_core::sync::graph_push::set_relay(self.0.store(), account, text)
        }

        /// `mail_core::sync::graph_push::relayed`.
        pub fn relayed(&self, account: &Configured) -> bool {
            mail_core::sync::graph_push::relayed(self.0.store(), account)
        }
    }
}
pub use graph_push::GraphPushCalls;

mod rules_manage {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::rules::manage::*;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::super::rules_manage::RulesManage<'_> {
        /// `mail_core::rules::manage::templates`.
        pub fn templates(&self, account: AccountId) -> Vec<String> {
            mail_core::rules::manage::templates(self.0.store(), account)
        }

        /// `mail_core::rules::manage::apply_to`.
        pub fn apply_to(
            &self,
            threads: &[mail_domain::ThreadId],
            now: DateTime<Utc>,
        ) -> Result<usize, CoreError> {
            mail_core::rules::manage::apply_to(self.0.store(), threads, now)
        }
    }
}
mod compose {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::compose::*;
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::super::compose::Compose<'_> {
        /// `mail_core::compose::sending_identities`.
        pub fn sending_identities(&self) -> Vec<Identity> {
            mail_core::compose::sending_identities(self.0.store())
        }
    }
}
mod compose_weight {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::compose::*;
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::super::compose::Compose<'_> {
        /// `mail_core::compose::send_limit`.
        pub fn send_limit(&self, account: &AccountId) -> SendLimit {
            mail_core::compose::send_limit(self.0.store(), account)
        }

        /// `mail_core::compose::can_ask_delivery_report`.
        pub fn can_ask_delivery_report(&self, account: &AccountId) -> bool {
            mail_core::compose::can_ask_delivery_report(self.0.store(), account)
        }

        /// `mail_core::compose::weigh`.
        pub fn weigh(&self, draft: &Draft) -> Weight {
            mail_core::compose::weigh(self.0.store(), draft)
        }
    }
}
mod compose_modes {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::compose::*;
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::super::compose::Compose<'_> {
        /// `mail_core::compose::redirect`.
        pub fn redirect(
            &self,
            message: MessageId,
            to: &[Address],
            grace: chrono::TimeDelta,
            now: DateTime<Utc>,
        ) -> Result<Redirected, CoreError> {
            mail_core::compose::redirect(self.0.store(), message, to, grace, now)
        }
    }
}
mod sync_caps {
    #![allow(clippy::too_many_arguments)]

    #[allow(unused_imports)]
    use chrono::{DateTime, Utc};
    #[allow(unused_imports)]
    use mail_core::error::CoreError;
    #[allow(unused_imports)]
    use mail_core::rich_text::Doc;
    #[allow(unused_imports)]
    use mail_core::sync::Configured;
    #[allow(unused_imports)]
    use mail_core::sync::*;
    #[allow(unused_imports)]
    use mail_domain::*;
    #[allow(unused_imports)]
    use mail_store::*;
    #[allow(unused_imports)]
    use porter_core::AccountId;

    impl super::super::sync::Sync<'_> {
        /// `mail_core::sync::caps_of`.
        pub fn caps_of(&self, account: AccountId) -> Option<AccountCaps> {
            mail_core::sync::caps_of(self.0.store(), account)
        }
    }
}
