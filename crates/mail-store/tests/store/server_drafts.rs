//! A draft's copy on the server, as the outbox holds it: one waiting upload per draft at most,
//! the latest, on both stores.
//!
//! Autosave asks for a copy as the draft changes, and offline the outbox keeps every request
//! until the server is back. Uploaded one by one, a draft written for an hour on a train would be
//! sent as dozens of whole messages, each replacing the last; only the last matters.

use crate::support::*;
use mail_domain::*;
use mail_store::{MemoryStore, Settle, Store};

fn drafts() -> MailboxRef {
    MailboxRef {
        account: acct_account(),
        path: "Drafts".to_owned(),
    }
}

fn keep(draft: DraftId, raw: BlobId) -> RemoteIntent {
    RemoteIntent::KeepDraft {
        draft,
        mailbox: drafts(),
        message_id: format!("{draft}@example.test"),
        raw,
    }
}

fn nothing() -> Patch {
    Patch {
        id: ChangeId::generate(),
        changes: Vec::new(),
    }
}

/// Both stores, the memory one knowing the account as the SQLite one does.
fn stores() -> Both {
    let both = Both::new();
    both.memory
        .import(acct_account(), Import { messages: vec![] })
        .unwrap();
    both
}

fn queued(store: &dyn Store) -> Vec<ProtoOp> {
    store
        .outbox_due(acct_account(), at(1_000_000))
        .unwrap()
        .into_iter()
        .map(|entry| entry.op)
        .collect()
}

#[test]
fn a_second_upload_of_a_draft_replaces_the_one_still_queued() {
    let both = stores();
    let (draft, other) = (DraftId::generate(), DraftId::generate());
    let (first, second, theirs) = (BlobId::generate(), BlobId::generate(), BlobId::generate());
    for store in [&both.sqlite as &dyn Store, &both.memory as &MemoryStore] {
        for (n, intent) in [keep(draft, first), keep(other, theirs), keep(draft, second)]
            .into_iter()
            .enumerate()
        {
            store
                .enqueue(acct_account(), intent, &nothing(), at(n as i64))
                .unwrap()
                .expect("a draft's copy addresses no message and is always queued");
        }
        let RemoteIntent::KeepDraft {
            mailbox,
            message_id,
            ..
        } = keep(draft, second)
        else {
            unreachable!()
        };
        let other_id = format!("{other}@example.test");
        assert_eq!(
            queued(store),
            vec![
                // Another draft's upload is its own, and stays where it was queued.
                ProtoOp::KeepDraft {
                    draft: other,
                    mailbox: drafts(),
                    message_id: other_id,
                    raw: theirs,
                },
                ProtoOp::KeepDraft {
                    draft,
                    mailbox,
                    message_id,
                    raw: second,
                },
            ]
        );
    }
}

#[test]
fn sending_or_discarding_a_draft_takes_its_waiting_upload_with_it() {
    let both = stores();
    let draft = DraftId::generate();
    let drop = RemoteIntent::DropDraft {
        draft,
        mailbox: drafts(),
        message_id: format!("{draft}@example.test"),
    };
    for store in [&both.sqlite as &dyn Store, &both.memory as &MemoryStore] {
        store
            .enqueue(
                acct_account(),
                keep(draft, BlobId::generate()),
                &nothing(),
                at(1),
            )
            .unwrap();
        store
            .enqueue(acct_account(), drop.clone(), &nothing(), at(2))
            .unwrap();
        assert_eq!(
            queued(store),
            vec![ProtoOp::DropDraft {
                draft,
                mailbox: drafts(),
                message_id: format!("{draft}@example.test"),
            }]
        );
    }
}

#[test]
fn an_upload_replaced_while_it_ran_settles_into_nothing() {
    let both = stores();
    let draft = DraftId::generate();
    let later = BlobId::generate();
    for store in [&both.sqlite as &dyn Store, &both.memory as &MemoryStore] {
        let running = store
            .enqueue(
                acct_account(),
                keep(draft, BlobId::generate()),
                &nothing(),
                at(1),
            )
            .unwrap()
            .unwrap();
        store
            .enqueue(acct_account(), keep(draft, later), &nothing(), at(2))
            .unwrap();
        // The drain that was running it answers as it would have, and the later one stays.
        store.outbox_settle(running, Settle::Ok, at(3)).unwrap();
        store
            .outbox_settle(
                running,
                Settle::Failed {
                    reason: "gone".to_owned(),
                    retry: Retry::Fatal("gone".to_owned()),
                },
                at(3),
            )
            .unwrap();
        assert!(
            matches!(&queued(store)[..], [ProtoOp::KeepDraft { raw, .. }] if *raw == later),
            "{:?}",
            queued(store)
        );
    }
}
