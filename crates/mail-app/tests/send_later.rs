//! Templates and send later from the command line (`plan.md` 10.8).
//!
//! `mailo send <draft> --at <when>` holds a send in the outbox until then; `mailo unsend` takes
//! it back while it waits; `mailo template …` keeps drafts to start from. Driven through
//! `cli::parse` and `cli::run_with_clients` against a real store, as a user would reach them.

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use mail_app::{cli, compose, template};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
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
        &mail_runtime::OAuthRegistry::default(),
    )
}

/// A store with one account that can send.
fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IDENTITY.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
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
        ACCOUNT,
        &someone(),
        "the plan",
        "see you there",
        now(),
    )
    .unwrap()
}

fn submissions(store: &SqliteStore) -> Vec<mail_store::OutboxEntry> {
    let far = now() + chrono::TimeDelta::try_days(3650).unwrap();
    store.outbox_due(ACCOUNT, far).unwrap()
}

mod parsing {
    use super::*;

    #[test]
    fn send_takes_a_time_in_the_words_snooze_takes() {
        let id = DraftId::generate();
        const CASES: &[(&str, Option<&str>)] = &[
            ("", None),
            (" --at tomorrow", Some("tomorrow")),
            (" --at +2h", Some("+2h")),
            (" --at 2026-09-25 09:00", Some("2026-09-25 09:00")),
        ];
        for (rest, at) in CASES {
            assert_eq!(
                cli::parse(&args(&format!("send {id}{rest}"))).unwrap(),
                cli::Command::Send {
                    draft: id,
                    at: at.map(str::to_owned),
                },
                "send {id}{rest}"
            );
        }
    }

    #[test]
    fn a_send_with_no_time_after_at_or_a_stray_option_is_refused() {
        let id = DraftId::generate();
        for line in [format!("send {id} --at"), format!("send {id} --later")] {
            assert!(cli::parse(&args(&line)).is_err(), "{line}");
        }
    }

    #[test]
    fn unsend_needs_a_draft() {
        let id = DraftId::generate();
        assert_eq!(
            cli::parse(&args(&format!("unsend {id}"))).unwrap(),
            cli::Command::Unsend { draft: id }
        );
        assert!(cli::parse(&args("unsend")).is_err());
        assert!(cli::parse(&args("unsend not-an-id")).is_err());
    }

    #[test]
    fn the_template_verbs() {
        let draft = DraftId::generate();
        let kept = TemplateId::generate();
        let cases: Vec<(String, cli::Command)> = vec![
            ("template".to_owned(), cli::Command::TemplateList),
            ("template list".to_owned(), cli::Command::TemplateList),
            (
                format!("template save {draft}"),
                cli::Command::TemplateSave {
                    draft,
                    name: String::new(),
                },
            ),
            (
                format!("template save {draft} weekly report"),
                cli::Command::TemplateSave {
                    draft,
                    name: "weekly report".to_owned(),
                },
            ),
            (
                format!("template use {kept}"),
                cli::Command::TemplateUse {
                    template: kept,
                    to: Vec::new(),
                },
            ),
            (
                format!("template use {kept} --to you@example.test"),
                cli::Command::TemplateUse {
                    template: kept,
                    to: someone(),
                },
            ),
            (
                format!("template delete {kept}"),
                cli::Command::TemplateDelete { template: kept },
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(cli::parse(&args(&line)).unwrap(), expected, "{line}");
        }
    }

    #[test]
    fn a_mistyped_template_command_is_explained() {
        let kept = TemplateId::generate();
        for line in [
            "template save".to_owned(),
            "template save not-an-id".to_owned(),
            format!("template use {kept} --to"),
            format!("template use {kept} --cc you@example.test"),
            "template delete".to_owned(),
            "template rename".to_owned(),
        ] {
            assert!(cli::parse(&args(&line)).is_err(), "{line}");
        }
    }

    #[test]
    fn usage_names_them() {
        let text = cli::usage();
        for needle in ["--at", "unsend", "template save", "template use"] {
            assert!(text.contains(needle), "usage does not mention {needle}");
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
                .outbox_due(ACCOUNT, leaves - chrono::TimeDelta::seconds(1))
                .unwrap()
                .is_empty(),
            "it would leave early"
        );
        assert_eq!(store.outbox_due(ACCOUNT, leaves).unwrap().len(), 1);
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
        assert_eq!(store.drafts(ACCOUNT).unwrap().len(), 3);
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
        let kept = store.templates(ACCOUNT).unwrap();
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
