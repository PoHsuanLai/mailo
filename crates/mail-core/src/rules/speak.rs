//! What a rule asks that only a running client can do: forward a message, answer it from a
//! template, or say it arrived ([`RuleAction::speaks`]).
//!
//! The store runs the rules and leaves these to its caller (`mail_store::rules::Ran::left`), so
//! that it never builds mail or raises a notification itself. A sync pass carries them out
//! before its outbox drain, so what they send leaves in the same pass.

use super::guard::{self, Occasion};
use crate::compose::Leaves;
use crate::error::{CoreError, Logged};
use crate::notify::Own;
use chrono::{DateTime, Utc};
use mail_domain::{Address, MessageId, ReplyScope, RuleAction};
use mail_store::rules::Left;
use mail_store::{SqliteStore, Store};

/// Carry out `left`, in order, and return the messages a rule asked to hear of, once each, for
/// the pass's notifications to say. A forward or reply that cannot be made — a template since
/// deleted, an account with no server to send from — is logged and the rest go on: one rule's
/// trouble is no reason to keep another's mail back. A forward or reply the guard refuses
/// ([`guard::guard`]) is not made, and is not an error.
pub fn carry_out(
    store: &SqliteStore,
    left: &[Left],
    occasion: Occasion,
    now: DateTime<Utc>,
) -> Vec<MessageId> {
    let mut noted = Vec::new();
    if left.is_empty() {
        return noted;
    }
    let own = crate::notify::own_addresses(store).or_log_default("the user's addresses");
    for one in left {
        match &one.action {
            RuleAction::Notify => {
                if !noted.contains(&one.message) {
                    noted.push(one.message);
                }
            }
            RuleAction::Forward(to) => {
                forward(store, one, to, &own, occasion, now)
                    .or_log("a rule could not forward a message");
            }
            RuleAction::Reply(template) => {
                reply(store, one, template, &own, occasion, now)
                    .or_log("a rule could not reply to a message");
            }
            _ => {}
        }
    }
    noted
}

/// Make `draft` the rule's, not a person's, and queue it.
fn send(
    store: &SqliteStore,
    mut draft: mail_domain::Draft,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    draft.automatic = true;
    crate::compose::save(store, &draft)?;
    crate::compose::queue(store, draft.id, Leaves::Now, now)?;
    Ok(())
}

fn forward(
    store: &SqliteStore,
    one: &Left,
    to: &str,
    own: &Own,
    occasion: Occasion,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let message = store.message(one.message)?;
    // Never back to whoever sent it: two people's rules forwarding to each other would loop.
    if message.from.email.eq_ignore_ascii_case(to.trim()) {
        return Ok(());
    }
    if guard::guard(store, one, &message, own, occasion, now).is_err() {
        return Ok(());
    }
    let draft =
        crate::compose::draft_forward(store, one.message, &[Address::new(to.trim())], "", now)?;
    send(store, draft, now)
}

fn reply(
    store: &SqliteStore,
    one: &Left,
    template: &str,
    own: &Own,
    occasion: Occasion,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let message = store.message(one.message)?;
    let kept = store
        .templates(message.account.clone())?
        .into_iter()
        .find(|kept| kept.name == template)
        .ok_or_else(|| CoreError::context(format!("template {template:?}"), "there is none"))?;
    // Last: it is what records the sender as answered.
    if guard::guard(store, one, &message, own, occasion, now).is_err() {
        return Ok(());
    }
    let draft =
        crate::compose::draft_reply(store, one.message, ReplyScope::Sender, &kept.text, now)?;
    send(store, draft, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support_tests::account;
    use chrono::TimeDelta;
    use mail_domain::{ReadState, RuleId, Star};

    const PERSON: &str = "Return-Path: <a@b.c>\r\n\r\nShall we?";

    fn left(message: MessageId, action: RuleAction, made: DateTime<Utc>) -> Left {
        Left {
            message,
            action,
            rule: RuleId::from_uuid(uuid::Uuid::from_u128(77)),
            made,
            reply_days: 4,
        }
    }

    fn queued(store: &SqliteStore) -> usize {
        mail_store::testing::outbox_ops(store).len()
    }

    fn setup(mail: &str) -> (SqliteStore, mail_domain::Message) {
        let store = crate::support_tests::sqlite_with(&[("Lunch", mail, 10)]);
        mail_store::testing::seed_default_identity(&store, account(), "me@example.test", None);
        let now = mail_domain::testing::now();
        let kept =
            crate::compose::draft_new(&store, account(), &[], "", "Back soon.", now).unwrap();
        crate::template::save(&store, kept.id, "Away", now).unwrap();
        let message =
            crate::support_tests::message(0, "Lunch", mail, 10, ReadState::Unread, Star::Unstarred);
        // The rule the lefts name: its replies are remembered against it.
        store
            .put_rule(&mail_domain::Rule {
                id: RuleId::from_uuid(uuid::Uuid::from_u128(77)),
                account: account(),
                name: "Away".to_owned(),
                position: 1,
                state: mail_domain::RuleState::Enabled,
                filter: mail_domain::Filter::All,
                actions: vec![RuleAction::Reply("Away".to_owned())],
                after: mail_domain::AfterMatch::Continue,
                made: now - TimeDelta::days(1),
                reply_days: 4,
            })
            .unwrap();
        (store, message)
    }

    /// A forward and a templated reply go into the outbox, a notification is handed back, and
    /// a reply to a template that is not there is left alone.
    #[test]
    fn what_rules_left_is_queued_or_handed_back() {
        let (store, message) = setup(PERSON);
        let now = mail_domain::testing::now();
        let made = now - TimeDelta::days(1);
        let left = vec![
            left(message.id, RuleAction::Notify, made),
            left(
                message.id,
                RuleAction::Forward("me@elsewhere.example".to_owned()),
                made,
            ),
            left(message.id, RuleAction::Reply("Away".to_owned()), made),
            left(
                message.id,
                RuleAction::Reply("No such template".to_owned()),
                made,
            ),
            left(message.id, RuleAction::Notify, made),
        ];
        let before = queued(&store);
        let noted = carry_out(&store, &left, Occasion::Arrival, now);
        assert_eq!(noted, vec![message.id], "said once");
        assert_eq!(queued(&store) - before, 2, "the forward and the reply");
    }

    /// Machines' mail, and mail from before the rule, send nothing; the person's own ask sends
    /// the older mail; the same sender is answered once in the rule's days.
    #[test]
    fn automatic_sends_keep_to_the_guard() {
        let now = mail_domain::testing::now();
        let made = now - TimeDelta::days(1);
        // A list's mail: no forward and no reply.
        let (store, message) = setup("Return-Path: <a@b.c>\r\nList-Id: <x.example>\r\n\r\nHi");
        let before = queued(&store);
        let both = vec![
            left(
                message.id,
                RuleAction::Forward("me@elsewhere.example".to_owned()),
                made,
            ),
            left(message.id, RuleAction::Reply("Away".to_owned()), made),
        ];
        carry_out(&store, &both, Occasion::Arrival, now);
        assert_eq!(queued(&store), before, "list mail is not answered");

        // Dated before the rule: not on arrival, yes when the person asked.
        let (store, message) = setup(PERSON);
        let before = queued(&store);
        let late = vec![left(
            message.id,
            RuleAction::Reply("Away".to_owned()),
            now + TimeDelta::days(1),
        )];
        carry_out(&store, &late, Occasion::Arrival, now);
        assert_eq!(queued(&store), before, "history is not answered");
        carry_out(&store, &late, Occasion::Asked, now);
        assert_eq!(queued(&store) - before, 1, "asked for, it is");

        // The same sender again within the rule's days: no second reply; after them: yes.
        carry_out(&store, &late, Occasion::Asked, now + TimeDelta::days(1));
        assert_eq!(queued(&store) - before, 1, "once in four days");
        carry_out(&store, &late, Occasion::Asked, now + TimeDelta::days(5));
        assert_eq!(queued(&store) - before, 2, "and again after");
    }

    /// What a rule sends is marked as a machine's.
    #[test]
    fn what_a_rule_queues_is_an_automatic_draft() {
        let (store, message) = setup(PERSON);
        let now = mail_domain::testing::now();
        let reply = vec![left(
            message.id,
            RuleAction::Reply("Away".to_owned()),
            now - TimeDelta::days(1),
        )];
        carry_out(&store, &reply, Occasion::Arrival, now);
        let drafts = store.drafts(account()).unwrap();
        assert!(
            drafts
                .iter()
                .any(|d| d.automatic && d.in_reply_to == Some(message.id)),
            "{drafts:?}"
        );
    }
}
