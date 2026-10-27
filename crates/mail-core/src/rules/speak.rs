//! What a rule asks that only a running client can do: forward a message, answer it from a
//! template, or say it arrived ([`RuleAction::speaks`]).
//!
//! The store runs the rules and leaves these to its caller (`mail_store::rules::Ran::left`), so
//! that it never builds mail or raises a notification itself. A sync pass carries them out
//! before its outbox drain, so what they send leaves in the same pass.

use crate::compose::Leaves;
use crate::error::{CoreError, Logged};
use crate::notify::Own;
use chrono::{DateTime, Utc};
use mail_domain::{Address, Message, MessageId, ReplyScope, RuleAction};
use mail_store::{SqliteStore, Store};

/// Carry out `left`, in order, and return the messages a rule asked to hear of, once each, for
/// the pass's notifications to say. A forward or reply that cannot be made — a template since
/// deleted, an account with no server to send from — is logged and the rest go on: one rule's
/// trouble is no reason to keep another's mail back.
pub fn carry_out(
    store: &SqliteStore,
    left: &[(MessageId, RuleAction)],
    now: DateTime<Utc>,
) -> Vec<MessageId> {
    let mut noted = Vec::new();
    if left.is_empty() {
        return noted;
    }
    let own = crate::notify::own_addresses(store).or_log_default("the user's addresses");
    for (id, action) in left {
        match action {
            RuleAction::Notify => {
                if !noted.contains(id) {
                    noted.push(*id);
                }
            }
            RuleAction::Forward(to) => {
                forward(store, *id, to, now).or_log("a rule could not forward a message");
            }
            RuleAction::Reply(template) => {
                reply(store, *id, template, &own, now)
                    .or_log("a rule could not reply to a message");
            }
            _ => {}
        }
    }
    noted
}

fn forward(
    store: &SqliteStore,
    id: MessageId,
    to: &str,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let message = store.message(id)?;
    // Never back to whoever sent it: two people's rules forwarding to each other would loop.
    if message.from.email.eq_ignore_ascii_case(to.trim()) {
        return Ok(());
    }
    let draft = crate::compose::draft_forward(store, id, &[Address::new(to.trim())], "", now)?;
    crate::compose::queue(store, draft.id, Leaves::Now, now)?;
    Ok(())
}

fn reply(
    store: &SqliteStore,
    id: MessageId,
    template: &str,
    own: &Own,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let message = store.message(id)?;
    if !answerable(&message, own) {
        return Ok(());
    }
    let kept = store
        .templates(message.account.clone())?
        .into_iter()
        .find(|kept| kept.name == template)
        .ok_or_else(|| CoreError::context(format!("template {template:?}"), "there is none"))?;
    let draft = crate::compose::draft_reply(store, id, ReplyScope::Sender, &kept.text, now)?;
    crate::compose::queue(store, draft.id, Leaves::Now, now)?;
    Ok(())
}

/// Whether a rule may answer `message`: not the user's own mail, and not an address that takes
/// no answers, which is where automatic mail — another client's rule among it — comes from.
fn answerable(message: &Message, own: &Own) -> bool {
    let from = message.from.email.trim();
    !from.is_empty() && !own.includes(&from.to_ascii_lowercase()) && !mail_store::no_reply(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support_tests::account;
    use mail_domain::{Body, MailboxRole, MessageKey, ReadState, Star, ThreadId};

    fn from(email: &str) -> Message {
        Message {
            id: MessageId::from_uuid(uuid::Uuid::from_u128(1)),
            thread: ThreadId::from_uuid(uuid::Uuid::from_u128(2)),
            account: account(),
            key: MessageKey::Rfc("m1@example.test".to_owned()),
            date: mail_domain::testing::now(),
            from: Address::new(email),
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "Hello".to_owned(),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: None,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Absent,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn a_rule_answers_people_and_not_machines_or_the_user() {
        let own = Own::new(["me@example.test"]);
        let cases = [
            ("ada@example.test", true),
            ("me@example.test", false),
            ("ME@example.test", false),
            ("noreply@bank.example", false),
            ("no-reply@shop.example", false),
            ("MAILER-DAEMON@mx.example", false),
            ("", false),
        ];
        for (email, want) in cases {
            assert_eq!(answerable(&from(email), &own), want, "{email}");
        }
    }

    /// A forward and a templated reply go into the outbox, a notification is handed back, and
    /// a reply to a machine, or to a template that is not there, is left alone.
    #[test]
    fn what_rules_left_is_queued_or_handed_back() {
        let store = crate::support_tests::sqlite_with(&[("Lunch", "Shall we?", 10)]);
        mail_store::testing::seed_default_identity(&store, account(), "me@example.test", None);
        let now = mail_domain::testing::now();
        let message = crate::support_tests::message(
            0,
            "Lunch",
            "Shall we?",
            10,
            ReadState::Unread,
            Star::Unstarred,
        );
        let kept =
            crate::compose::draft_new(&store, account(), &[], "", "Back soon.", now).unwrap();
        crate::template::save(&store, kept.id, "Away", now).unwrap();
        let left = vec![
            (message.id, RuleAction::Notify),
            (
                message.id,
                RuleAction::Forward("me@elsewhere.example".to_owned()),
            ),
            (message.id, RuleAction::Reply("Away".to_owned())),
            (message.id, RuleAction::Reply("No such template".to_owned())),
            (message.id, RuleAction::Notify),
        ];
        let queued = mail_store::testing::outbox_ops(&store).len();
        let noted = carry_out(&store, &left, now);
        assert_eq!(noted, vec![message.id], "said once");
        let after = mail_store::testing::outbox_ops(&store).len();
        assert_eq!(after - queued, 2, "the forward and the reply");
    }
}
