//! Templates and send later from the command line (`plan.md` 10.8).
//!
//! `mailo send <draft> --at <when>` holds a send in the outbox until then; `mailo unsend` takes
//! it back while it waits; `mailo template …` keeps drafts to start from. Driven through
//! `cli::parse` and `cli::run_with_clients` against a real store, as a user would reach them.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use mail_app::cli;
use mail_core::compose;
use mail_core::template;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

/// Thursday 24 September 2026, 17:00 UTC.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 17, 0, 0).unwrap()
}

fn utc() -> FixedOffset {
    FixedOffset::east_opt(0).unwrap()
}

fn args(s: &str) -> Vec<String> {
    s.split(' ').map(str::to_owned).collect()
}

fn exercise(store: &SqliteStore, command: &cli::Command) -> Result<String, String> {
    cli::run_with_clients(
        store,
        command,
        now(),
        &mail_runtime::ClientRegistry::default(),
    )
}

/// A store with one account that can send.
fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    {
        mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
        mail_store::testing::seed_identity_for(
            &store,
            IDENTITY,
            acct_account(),
            "me@example.test",
            None,
        );
    }
    (store, dir)
}

fn someone() -> Vec<Address> {
    vec![Address {
        name: None,
        email: "you@example.test".to_owned(),
    }]
}

fn a_draft(store: &SqliteStore) -> Draft {
    compose::draft_new(
        store,
        acct_account(),
        &someone(),
        "the plan",
        "see you there",
        now(),
    )
    .unwrap()
}

fn submissions(store: &SqliteStore) -> Vec<mail_store::OutboxEntry> {
    let far = now() + chrono::TimeDelta::try_days(3650).unwrap();
    store.outbox_due(acct_account(), far).unwrap()
}

mod parsing {
    use super::*;

    #[test]
    fn send_and_unsend_parse() {
        enum Want {
            Send(Option<&'static str>),
            Unsend,
            Refused(&'static str),
        }
        // `--at` takes the rest of the line, in the words snooze takes. `{id}` is a draft id.
        const CASES: &[(&str, &str, Want)] = &[
            ("send now", "send {id}", Want::Send(None)),
            (
                "send at a word",
                "send {id} --at tomorrow",
                Want::Send(Some("tomorrow")),
            ),
            (
                "send at an offset",
                "send {id} --at +2h",
                Want::Send(Some("+2h")),
            ),
            (
                "send at a date and time",
                "send {id} --at 2026-09-25 09:00",
                Want::Send(Some("2026-09-25 09:00")),
            ),
            (
                "no time after --at",
                "send {id} --at",
                Want::Refused("--at needs a time: mailo send {id} --at tomorrow"),
            ),
            (
                "a stray option",
                "send {id} --later",
                Want::Refused("unknown option \"--later\""),
            ),
            ("unsend", "unsend {id}", Want::Unsend),
            (
                "unsend with no draft",
                "unsend",
                Want::Refused("unsend needs a draft id"),
            ),
            (
                "unsend a bad id",
                "unsend not-an-id",
                Want::Refused("\"not-an-id\" is not a draft id"),
            ),
        ];
        let id = DraftId::generate();
        let fill = |text: &str| text.replace("{id}", &id.to_string());
        for (name, line, want) in CASES {
            let parsed = cli::parse(&args(&fill(line)));
            match want {
                Want::Send(at) => assert_eq!(
                    parsed,
                    Ok(cli::Command::Send {
                        draft: id,
                        at: at.map(str::to_owned),
                    }),
                    "{name}"
                ),
                Want::Unsend => {
                    assert_eq!(parsed, Ok(cli::Command::Unsend { draft: id }), "{name}")
                }
                Want::Refused(said) => {
                    let err = parsed.expect_err(name);
                    assert!(err.starts_with(&fill(said)), "{name}: {err}");
                }
            }
        }
    }

    #[test]
    fn the_template_verbs_parse() {
        let draft = DraftId::generate();
        let kept = TemplateId::generate();
        let refused = |said: &str| Err::<cli::Command, _>(said.to_owned());
        let cases: Vec<(&str, String, Result<cli::Command, String>)> = vec![
            (
                "list, bare",
                "template".to_owned(),
                Ok(cli::Command::TemplateList),
            ),
            (
                "list",
                "template list".to_owned(),
                Ok(cli::Command::TemplateList),
            ),
            (
                "save with no name",
                format!("template save {draft}"),
                Ok(cli::Command::TemplateSave {
                    draft,
                    name: String::new(),
                }),
            ),
            (
                "save with a name of several words",
                format!("template save {draft} weekly report"),
                Ok(cli::Command::TemplateSave {
                    draft,
                    name: "weekly report".to_owned(),
                }),
            ),
            (
                "use",
                format!("template use {kept}"),
                Ok(cli::Command::TemplateUse {
                    template: kept,
                    to: Vec::new(),
                }),
            ),
            (
                "use with recipients",
                format!("template use {kept} --to you@example.test"),
                Ok(cli::Command::TemplateUse {
                    template: kept,
                    to: someone(),
                }),
            ),
            (
                "delete",
                format!("template delete {kept}"),
                Ok(cli::Command::TemplateDelete { template: kept }),
            ),
            (
                "save with no draft",
                "template save".to_owned(),
                refused("usage: mailo template save <draft-id> [name]"),
            ),
            (
                "save a bad id",
                "template save not-an-id".to_owned(),
                refused("\"not-an-id\" is not a draft id"),
            ),
            (
                "use with no address after --to",
                format!("template use {kept} --to"),
                refused("usage: mailo template use <template-id> [--to a@b[,c@d]]"),
            ),
            (
                "use with --cc",
                format!("template use {kept} --cc you@example.test"),
                refused("usage: mailo template use <template-id> [--to a@b[,c@d]]"),
            ),
            (
                "delete with no template",
                "template delete".to_owned(),
                refused("usage: mailo template delete <template-id>"),
            ),
            (
                "an unknown verb",
                "template rename".to_owned(),
                refused("unknown template command \"rename\""),
            ),
        ];
        for (name, line, want) in cases {
            match (cli::parse(&args(&line)), want) {
                (Err(err), Err(said)) => assert!(err.starts_with(&said), "{name}: {err}"),
                (got, want) => assert_eq!(got, want, "{name}"),
            }
        }
    }
}

mod sending_later {
    use super::*;

    #[test]
    fn a_send_at_a_time_is_held_until_then_and_the_draft_says_when() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        let out =
            compose::send_later_in(&store, draft.id, "2026-09-25 09:00", now(), &utc()).unwrap();
        assert!(out.contains("2026-09-25 09:00"), "{out}");
        assert!(out.contains(&format!("mailo unsend {}", draft.id)), "{out}");

        let leaves = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
        assert_eq!(
            store.draft(draft.id).unwrap().state,
            SendState::Scheduled { at: leaves }
        );
        assert!(
            store
                .outbox_due(acct_account(), leaves - chrono::TimeDelta::seconds(1))
                .unwrap()
                .is_empty(),
            "it would leave early"
        );
        assert_eq!(store.outbox_due(acct_account(), leaves).unwrap().len(), 1);
    }

    #[test]
    fn the_command_reads_the_time_the_way_snooze_does() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        exercise(
            &store,
            &cli::Command::Send {
                draft: draft.id,
                at: Some("+2h".to_owned()),
            },
        )
        .unwrap();
        assert_eq!(
            store.draft(draft.id).unwrap().state,
            SendState::Scheduled {
                at: now() + chrono::TimeDelta::try_hours(2).unwrap()
            }
        );
    }

    #[test]
    fn drafts_lists_it_as_scheduled_with_its_time() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        compose::send_later_in(&store, draft.id, "2026-09-25 09:00", now(), &utc()).unwrap();
        let listed = compose::drafts_in(&store, &utc()).unwrap();
        assert!(listed.contains("scheduled"), "{listed}");
        assert!(listed.contains("leaves 2026-09-25 09:00"), "{listed}");
    }

    #[test]
    fn a_time_that_has_passed_is_refused_and_an_existing_schedule_kept() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        compose::send_later_in(&store, draft.id, "2026-09-25 09:00", now(), &utc()).unwrap();
        let err = compose::send_later_in(&store, draft.id, "2026-09-01", now(), &utc())
            .expect_err("the past");
        assert!(err.contains("already passed"), "{err}");
        assert!(matches!(
            store.draft(draft.id).unwrap().state,
            SendState::Scheduled { .. }
        ));
        assert_eq!(submissions(&store).len(), 1);
    }

    #[test]
    fn scheduling_again_moves_it_rather_than_sending_it_twice() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        compose::send_later_in(&store, draft.id, "2026-09-25 09:00", now(), &utc()).unwrap();
        compose::send_later_in(&store, draft.id, "2026-09-26 09:00", now(), &utc()).unwrap();
        let queued = submissions(&store);
        assert_eq!(queued.len(), 1, "two submissions of one message");
        assert_eq!(
            queued[0].next_attempt,
            Utc.with_ymd_and_hms(2026, 9, 26, 9, 0, 0).unwrap()
        );
    }

    #[test]
    fn sending_a_queued_draft_again_leaves_one_submission() {
        // Before this, a second `mailo send` put a second submission beside the first, and the
        // recipient got the message twice.
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        compose::send(&store, draft.id, now()).unwrap();
        compose::send(&store, draft.id, now()).unwrap();
        assert_eq!(submissions(&store).len(), 1);
        // And sending now a message that was scheduled sends it now.
        compose::send_later_in(&store, draft.id, "tomorrow", now(), &utc()).unwrap();
        compose::send(&store, draft.id, now()).unwrap();
        let queued = submissions(&store);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].next_attempt, now());
        assert_eq!(store.draft(draft.id).unwrap().state, SendState::Queued);
    }

    #[test]
    fn unsend_takes_a_scheduled_send_back_to_an_editable_draft() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        compose::send_later_in(&store, draft.id, "tomorrow", now(), &utc()).unwrap();
        let out = exercise(&store, &cli::Command::Unsend { draft: draft.id }).unwrap();
        assert!(out.contains("draft again"), "{out}");
        assert!(submissions(&store).is_empty(), "it would still have gone");
        let back = store.draft(draft.id).unwrap();
        assert_eq!(back.state, SendState::Editing);
        assert_eq!(back.subject, draft.subject);
        assert_eq!(back.text, draft.text);
        // Editable again: attaching is refused only for a send on its way.
        compose::attach_bytes(&store, draft.id, "notes.txt", b"hello", now()).unwrap();
    }

    #[test]
    fn a_scheduled_send_cannot_be_edited_or_discarded_underneath_the_outbox() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        compose::send_later_in(&store, draft.id, "tomorrow", now(), &utc()).unwrap();
        assert!(compose::attach_bytes(&store, draft.id, "late.txt", b"x", now()).is_err());
        assert!(compose::discard(&store, draft.id).is_err());
        assert_eq!(submissions(&store).len(), 1);
    }

    #[test]
    fn once_it_is_on_the_wire_it_cannot_be_taken_back() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);
        compose::send_later_in(&store, draft.id, "tomorrow", now(), &utc()).unwrap();
        // What the engine writes as it hands the message over.
        store
            .set_send_state(draft.id, &SendState::Sending, now())
            .unwrap();
        let err = exercise(&store, &cli::Command::Unsend { draft: draft.id }).unwrap_err();
        assert!(err.contains("already being sent"), "{err}");
        assert_eq!(submissions(&store).len(), 1);
    }
}

mod templates {
    use super::*;

    #[test]
    fn a_draft_kept_as_a_template_starts_new_drafts_and_stays() {
        let (store, _dir) = seeded();
        let mut draft = a_draft(&store);
        draft.cc = vec![Address {
            name: None,
            email: "lead@example.test".to_owned(),
        }];
        compose::save(&store, &draft).unwrap();
        let draft =
            compose::attach_bytes(&store, draft.id, "plan.txt", b"the plan", now()).unwrap();

        let kept = template::save(&store, draft.id, "weekly", now()).unwrap();
        assert_eq!(kept.name, "weekly");
        assert!(store.draft(draft.id).is_ok(), "the draft stays a draft");

        let first = template::start(&store, kept.id, &[], now()).unwrap();
        let second = template::start(&store, kept.id, &[], now()).unwrap();
        assert_ne!(first.id, second.id);
        for started in [&first, &second] {
            let stored = store.draft(started.id).unwrap();
            assert_eq!(stored.to, draft.to);
            assert_eq!(stored.cc, draft.cc);
            assert_eq!(stored.subject, draft.subject);
            assert_eq!(stored.text, draft.text);
            assert_eq!(stored.attachments, draft.attachments);
            assert_eq!(stored.identity, draft.identity);
            assert_eq!(stored.state, SendState::Editing);
        }
        assert_eq!(store.template(kept.id).unwrap(), kept, "the template moved");
        // Templates are not drafts, and do not show up among them.
        assert_eq!(store.drafts(acct_account()).unwrap().len(), 3);
    }

    #[test]
    fn to_replaces_the_templates_own_recipients() {
        let (store, _dir) = seeded();
        let kept = template::save(&store, a_draft(&store).id, "", now()).unwrap();
        let other = vec![Address {
            name: None,
            email: "someone-else@example.test".to_owned(),
        }];
        let started = template::start(&store, kept.id, &other, now()).unwrap();
        assert_eq!(started.to, other);
    }

    #[test]
    fn a_draft_from_a_template_can_be_sent() {
        let (store, _dir) = seeded();
        let kept = template::save(&store, a_draft(&store).id, "", now()).unwrap();
        let started = template::start(&store, kept.id, &[], now()).unwrap();
        compose::send(&store, started.id, now()).unwrap();
        assert_eq!(submissions(&store).len(), 1);
    }

    #[test]
    fn through_the_command_line() {
        let (store, _dir) = seeded();
        let draft = a_draft(&store);

        let saved = exercise(
            &store,
            &cli::Command::TemplateSave {
                draft: draft.id,
                name: "weekly report".to_owned(),
            },
        )
        .unwrap();
        let kept = store.templates(acct_account()).unwrap();
        assert_eq!(kept.len(), 1);
        assert!(saved.contains(&kept[0].id.to_string()), "{saved}");

        let listed = exercise(&store, &cli::Command::TemplateList).unwrap();
        assert!(listed.contains("weekly report"), "{listed}");
        assert!(listed.contains(&kept[0].id.to_string()), "{listed}");

        let used = exercise(
            &store,
            &cli::Command::TemplateUse {
                template: kept[0].id,
                to: Vec::new(),
            },
        )
        .unwrap();
        // The new draft's id comes first, so a script can take it from the first line.
        let first = used.lines().next().unwrap();
        let id: uuid::Uuid = first.strip_prefix("draft ").unwrap().parse().unwrap();
        assert!(store.draft(DraftId::from_uuid(id)).is_ok(), "{used}");

        let deleted = exercise(
            &store,
            &cli::Command::TemplateDelete {
                template: kept[0].id,
            },
        )
        .unwrap();
        assert!(deleted.contains("weekly report"), "{deleted}");
        assert!(
            exercise(&store, &cli::Command::TemplateList)
                .unwrap()
                .contains("no templates")
        );
        assert!(
            exercise(
                &store,
                &cli::Command::TemplateDelete {
                    template: kept[0].id,
                },
            )
            .is_err(),
            "deleting it twice is not reported as done"
        );
    }
}
