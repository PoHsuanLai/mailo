//! Writing mail: a draft, and a message sent.
//!
//! A draft is saved and sent by nobody, so making one is a write that can be taken back (the
//! draft is discarded). A send is outbound: mailo queues it as it queues one from the window, and
//! the person can take it back to a draft until the outbox has delivered it.

use super::{Provider, outcome, token::Token};
use crate::intents::APP;
use crate::intents::wire::{
    AppRefusal, EntityId, Invocation, Label, Labelled, Outcome, Output, Preview, UndoFault,
};
use chrono::Utc;
use mail_core::compose;
use mail_domain::{AccountId, Address, Draft, DraftId};

/// The addresses in `text` (a comma-separated list, as a person types it), or why not.
fn addresses(text: Option<&str>) -> Result<Vec<Address>, AppRefusal> {
    compose::addresses::parse_addresses(text.unwrap_or_default()).map_err(AppRefusal::Failed)
}

/// An address as a person reads it.
fn shown(address: &Address) -> String {
    compose::addresses::join_addresses(std::slice::from_ref(address))
}

impl Provider {
    /// The account a message leaves from: the one the caller named, or the only one there is.
    fn sender(&self, invocation: &Invocation) -> Result<AccountId, AppRefusal> {
        let wanted = invocation.text("from");
        if wanted.is_none() && compose::sending_accounts(&self.store).len() > 1 {
            return Err(AppRefusal::NeedsParam {
                param: "from".to_owned(),
                options: Vec::new(),
            });
        }
        compose::account_for(&self.store, wanted).map_err(AppRefusal::Failed)
    }

    /// Save a draft with what the caller gave: `to`, `subject` and `body`, any of them.
    pub(super) fn create_draft(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        let to = addresses(invocation.text("to"))?;
        let draft = compose::draft_new(
            &self.store,
            self.sender(invocation)?,
            &to,
            invocation.text("subject").unwrap_or_default(),
            invocation.text("body").unwrap_or_default(),
            Utc::now(),
        )
        .map_err(AppRefusal::Failed)?;
        let id = EntityId {
            app: APP.to_owned(),
            kind: "mail.draft".to_owned(),
            key: draft.id.to_string(),
        };
        let created = Labelled {
            value: Output::Entities(vec![id]),
            label: Label::own(APP),
        };
        Ok(outcome(
            Some("Draft saved".to_owned()),
            Some(Token::Discard(draft.id)),
            Some(created),
        ))
    }

    /// Make a message of what the caller gave and queue it for delivery. A message that cannot be
    /// queued leaves no draft behind.
    pub(super) fn send(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        let to = addresses(invocation.text("to"))?;
        if to.is_empty() {
            return Err(missing("to"));
        }
        let body = invocation.text("body").ok_or_else(|| missing("body"))?;
        let draft = compose::draft_new(
            &self.store,
            self.sender(invocation)?,
            &to,
            invocation.text("subject").unwrap_or_default(),
            body,
            Utc::now(),
        )
        .map_err(AppRefusal::Failed)?;
        let queued = compose::send_with(
            &self.store,
            self.secrets.as_ref(),
            &mail_core::pgp::no_passphrase,
            draft.id,
            Utc::now(),
        );
        if let Err(why) = queued {
            let _ = compose::discard(&self.store, draft.id);
            return Err(AppRefusal::Failed(why.to_string()));
        }
        Ok(outcome(
            Some("Queued for delivery".to_owned()),
            Some(Token::Unsend(draft.id)),
            None,
        ))
    }

    /// What `send` would send, for the person to read before it does.
    pub(super) fn preview_send(&self, invocation: &Invocation) -> Result<Preview, AppRefusal> {
        let own = || Label::own(APP);
        let say = |value: String| Labelled {
            value,
            label: own(),
        };
        Ok(Preview::Message {
            to: addresses(invocation.text("to"))?
                .iter()
                .map(|address| say(shown(address)))
                .collect(),
            subject: say(invocation.text("subject").unwrap_or_default().to_owned()),
            body: say(invocation.text("body").unwrap_or_default().to_owned()),
        })
    }

    /// Discard a draft an action made.
    pub(super) fn discard(&self, draft: DraftId) -> Result<(), UndoFault> {
        compose::discard(&self.store, draft)
            .map(drop)
            .map_err(|_| UndoFault::Gone)
    }

    /// Take a queued message back to a draft.
    pub(super) fn unsend(&self, draft: DraftId) -> Result<(), UndoFault> {
        compose::unsend(&self.store, draft, Utc::now())
            .map(|_: Draft| ())
            .map_err(|_| UndoFault::Gone)
    }
}

fn missing(param: &str) -> AppRefusal {
    AppRefusal::NeedsParam {
        param: param.to_owned(),
        options: Vec::new(),
    }
}
