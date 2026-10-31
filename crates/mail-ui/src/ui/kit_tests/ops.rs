//! What the window's writes ask of the server, over the window's fixtures.

mod tests {
    use crate::ui::fixtures::{acct_account, gmail_caps, inbox_query, realistic};
    use mail_core::{SqliteStore, Store};
    use mail_domain::*;
    use mail_ui_kit::ops::apply_op;

    /// What the server is told when the window acts.
    ///
    /// `Op::apply` returns the local patch, its undo, *and* the remote work. `apply_op` used
    /// the first and dropped the third, and built the `AccountCaps` it passed out of safe
    /// defaults rather than reading the account's own — and `remote_intent` is the only
    /// consumer of `caps`, returning `None` for Archive and Trash under `LocalOnly`. So the
    /// operation could not have produced remote work, and would have been discarded if it had.
    mod what_the_server_is_told {
        use super::*;

        fn first_thread(client: &SqliteStore) -> ThreadId {
            client
                .threads(&inbox_query(), chrono::Utc::now())
                .unwrap()
                .items
                .first()
                .expect("the fixture has mail")
                .id
        }

        #[test]
        fn archiving_in_the_window_reaches_the_server() {
            // The conversation used to be archived here and nowhere else: still in the inbox on
            // the phone, and back in this one on the next full sync.
            let (client, _dir) = realistic();
            let thread = first_thread(&client);

            assert!(apply_op(
                &crate::ui::fixtures::client(&client),
                thread,
                OpKind::Archive
            ));

            let queued = client
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            let intents: Vec<&ProtoOp> = queued.iter().map(|entry| &entry.op).collect();
            assert!(
                intents.iter().any(|op| matches!(
                    op,
                    ProtoOp::SetMailbox {
                        role: MailboxRole::Archive,
                        ..
                    }
                )),
                "nothing was queued for the server: {intents:?}"
            );
        }

        #[test]
        fn marking_read_reaches_the_server_too() {
            // This one produces a remote intent whatever the capabilities say, so it was lost
            // purely to the dropped field — mail read here stayed bold everywhere else.
            let (client, _dir) = realistic();
            let thread = first_thread(&client);

            assert!(apply_op(
                &crate::ui::fixtures::client(&client),
                thread,
                OpKind::MarkRead
            ));

            let queued = client
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            assert!(
                queued
                    .iter()
                    .any(|entry| matches!(&entry.op, ProtoOp::SetFlags { read: Some(_), .. })),
                "the flag never left this machine: {:?}",
                queued.iter().map(|e| &e.op).collect::<Vec<_>>()
            );
        }

        #[test]
        fn an_account_that_files_nothing_queues_nothing() {
            // The control, and the reason this was invisible: on a POP3 account
            // `ArchiveMeans::LocalOnly` is the truth and archiving really is local. The account
            // tested against most was the one where the hardcoded value happened to be right.
            let (client, _dir) = realistic();
            let local_only = AccountCaps {
                labels: ServerLabels::LocalOnly,
                archive: ArchiveMeans::LocalOnly,
                ..gmail_caps()
            };
            mail_store::testing::seed_caps(
                &client,
                acct_account(),
                &local_only,
                chrono::Utc::now(),
            )
            .unwrap();
            let thread = first_thread(&client);

            assert!(apply_op(
                &crate::ui::fixtures::client(&client),
                thread,
                OpKind::Archive
            ));
            let queued = client
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            assert!(
                !queued
                    .iter()
                    .any(|entry| matches!(&entry.op, ProtoOp::SetMailbox { .. })),
                "a POP3 account has nowhere to file anything"
            );
        }

        #[test]
        fn a_pin_has_no_server_half_and_queues_nothing() {
            // Pin and snooze are this application's own state. Queueing them would ask the
            // server to do something it has no word for.
            let (client, _dir) = realistic();
            let thread = first_thread(&client);

            assert!(apply_op(
                &crate::ui::fixtures::client(&client),
                thread,
                OpKind::Pin
            ));
            let queued = client
                .outbox_due(acct_account(), chrono::Utc::now())
                .unwrap();
            assert!(
                queued.is_empty(),
                "{:?}",
                queued.iter().map(|e| &e.op).collect::<Vec<_>>()
            );
        }
    }
}
