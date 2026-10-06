//! Writing mail: a draft, a message sent, conversations forwarded.
//!
//! A draft is saved and sent by nobody, so making one is a write that can be taken back (the
//! draft is discarded). A send or a forward is outbound: mailo queues it as it queues one from the
//! window, and the person can take it back to a draft until the outbox has delivered it.
//!
//! What a preview shows of an argument carries the argument's label on: a recipient an agent
//! lifted from a message is drawn as somebody else's words on the confirmation sheet, not as
//! mailo's.

use super::{Provider, outcome, threads::threads_of, token::Token};
use crate::intents::APP;
use crate::intents::wire::{
    AppRefusal, EntityId, Invocation, Label, Labelled, Outcome, Output, Preview, UndoFault,
};
use chrono::{Local, Utc};
use mail_core::compose;
use mail_domain::{Address, Draft, DraftId, MessageId, SendState};
use mail_store::Store;
use porter_core::AccountId;

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
            Some(Token::Unsend(vec![draft.id])),
            None,
        ))
    }

    /// What `send` would send, for the person to read before it does, each part under the
    /// label its argument came with.
    pub(super) fn preview_send(&self, invocation: &Invocation) -> Result<Preview, AppRefusal> {
        let said = |param: &str, value: String| Labelled {
            value,
            label: invocation
                .label(param)
                .cloned()
                .unwrap_or_else(|| Label::own(APP)),
        };
        Ok(Preview::Message {
            to: addresses(invocation.text("to"))?
                .iter()
                .map(|address| said("to", shown(address)))
                .collect(),
            subject: said(
                "subject",
                invocation.text("subject").unwrap_or_default().to_owned(),
            ),
            body: said(
                "body",
                invocation.text("body").unwrap_or_default().to_owned(),
            ),
        })
    }

    /// The newest message of every conversation a forward names, and the contact it goes to, or
    /// why not. Asked of every one before anything is made.
    fn forwarding(
        &self,
        invocation: &Invocation,
    ) -> Result<(Vec<MessageId>, Labelled<Address>), AppRefusal> {
        let to = invocation.entity("to").ok_or_else(|| missing("to"))?;
        // Who it goes to is the address book's, and which of them was the caller's choice: the
        // label is both.
        let to = Labelled {
            value: self.contact_address(to)?,
            label: invocation.label("to").map_or_else(
                || Label::contacts(&invocation.space),
                |chosen| chosen.join(&Label::contacts(&invocation.space)),
            ),
        };
        let newest = threads_of(invocation)?
            .into_iter()
            .map(|(id, thread)| {
                self.store
                    .thread(thread)
                    .ok()
                    .and_then(|loaded| loaded.messages.last().copied())
                    .ok_or(AppRefusal::NotFound(id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if newest.is_empty() {
            return Err(AppRefusal::Unsupported);
        }
        Ok((newest, to))
    }

    /// Forward the newest message of every conversation named to one contact, each as a message
    /// of its own from the account it arrived at, and queue them. All are queued or none is: one
    /// that cannot be takes back the ones before it.
    pub(super) fn forward(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        let (newest, to) = self.forwarding(invocation)?;
        let mut queued: Vec<DraftId> = Vec::new();
        for message in newest {
            let made = compose::draft_forward(
                &self.store,
                message,
                std::slice::from_ref(&to.value),
                "",
                Utc::now(),
            );
            let sent = made.and_then(|draft| {
                compose::send_with(
                    &self.store,
                    self.secrets.as_ref(),
                    &mail_core::pgp::no_passphrase,
                    draft.id,
                    Utc::now(),
                )
                .map(|_| draft.id)
                .map_err(|why| {
                    let _ = compose::discard(&self.store, draft.id);
                    why.to_string()
                })
            });
            match sent {
                Ok(draft) => queued.push(draft),
                Err(why) => {
                    for draft in queued {
                        if compose::unsend(&self.store, draft, Utc::now()).is_ok() {
                            let _ = compose::discard(&self.store, draft);
                        }
                    }
                    return Err(AppRefusal::Failed(why));
                }
            }
        }
        let said = match queued.len() {
            1 => "Forward queued for delivery".to_owned(),
            n => format!("{n} forwards queued for delivery"),
        };
        Ok(outcome(Some(said), Some(Token::Unsend(queued)), None))
    }

    /// What `forward` would send: to whom, and the forwards themselves. The forwarded words are
    /// the mail's, so they are labelled as mail; several forwards show as one, one after another.
    pub(super) fn preview_forward(&self, invocation: &Invocation) -> Result<Preview, AppRefusal> {
        let (newest, to) = self.forwarding(invocation)?;
        let made = newest
            .into_iter()
            .map(|message| {
                compose::forward_unsaved_in(
                    &self.store,
                    message,
                    std::slice::from_ref(&to.value),
                    "",
                    Utc::now(),
                    &Local,
                )
            })
            .collect::<Result<Vec<Draft>, String>>()
            .map_err(AppRefusal::Failed)?;
        let theirs = |value: String| Labelled {
            value,
            label: Label::mail(&invocation.space),
        };
        Ok(Preview::Message {
            to: vec![Labelled {
                value: shown(&to.value),
                label: to.label,
            }],
            subject: theirs(
                made.iter()
                    .map(|draft| draft.subject.as_str())
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
            body: theirs(
                made.iter()
                    .map(|draft| draft.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
        })
    }

    /// Discard a draft an action made.
    pub(super) fn discard(&self, draft: DraftId) -> Result<(), UndoFault> {
        compose::discard(&self.store, draft)
            .map(drop)
            .map_err(|_| UndoFault::Gone)
    }

    /// Take queued messages back to drafts. None left to take back (delivered, or taken back
    /// already) is `Gone`; some but not all is a `Conflict`.
    pub(super) fn unsend(&self, drafts: &[DraftId]) -> Result<(), UndoFault> {
        let waiting = |draft: &DraftId| {
            self.store
                .draft(*draft)
                .is_ok_and(|stored| stored.state != SendState::Editing)
        };
        let back = drafts
            .iter()
            .filter(|draft| {
                waiting(draft)
                    && compose::unsend(&self.store, **draft, Utc::now())
                        .map(|_: Draft| ())
                        .is_ok()
            })
            .count();
        match back {
            0 => Err(UndoFault::Gone),
            n if n == drafts.len() => Ok(()),
            _ => Err(UndoFault::Conflict),
        }
    }
}

fn missing(param: &str) -> AppRefusal {
    AppRefusal::NeedsParam {
        param: param.to_owned(),
        options: Vec::new(),
    }
}
