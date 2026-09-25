use super::super::app::App;
use super::super::menu::Right;
use super::super::ops::apply_label;
use crate::ui::fixtures::{ACCOUNT, dispatching, inbox_query, markup, realistic};
use chrono::TimeZone;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// Putting a conversation off, from the window — `plan.md` phase 7e.
///
/// The Snoozed place has listed correctly since the place existed and the vocabulary has
/// been parsed since the CLI learned it. Nothing in the window could snooze anything.
mod putting_it_off {
    use super::*;

    #[tokio::test]
    async fn the_rows_offer_a_way_to_snooze() {
        let (store, _dir) = realistic();
        // The strip is an icon. The name is the accessible label; the fly says when.
        assert!(markup(store).contains("aria-label=\"Snooze\""));
    }

    #[test]
    fn snoozing_takes_it_out_of_the_inbox_and_the_snoozed_place_has_it() {
        let (store, _dir) = realistic();
        // The *place's* filter, not a bare `InMailbox`: hiding a snoozed conversation is
        // what `place_filter` is for, and asserting against the bare one would be asking
        // whether snoozing archives things, which it does not.
        let place = Query {
            filter: crate::view::place_filter(MailboxRole::Inbox),
            ..inbox_query()
        };
        let before = store.threads(&place, chrono::Utc::now()).unwrap();
        let thread = before.items[0].id;

        crate::snooze::snooze(&store, thread, "tomorrow", chrono::Utc::now())
            .expect("tomorrow is a time");

        let after = store.threads(&place, chrono::Utc::now()).unwrap();
        assert!(
            !after.items.iter().any(|t| t.id == thread),
            "a snoozed conversation is still in the inbox"
        );
        let asleep = store
            .threads(
                &Query {
                    filter: crate::view::pending_snooze(),
                    ..inbox_query()
                },
                chrono::Utc::now(),
            )
            .unwrap();
        assert!(asleep.items.iter().any(|t| t.id == thread), "{asleep:?}");
    }

    #[test]
    fn every_phrase_the_menu_offers_is_one_the_parser_accepts() {
        // The menu's phrases are the command line's, so a button that said something the
        // parser had never heard of would be a button that does nothing. Checked rather
        // than assumed, because the two lists are written in different files.
        let now = chrono::Utc::now();
        for (says, phrase) in crate::view::snooze_choices() {
            let at = crate::view::snooze_until(phrase, now, &chrono::Local)
                .unwrap_or_else(|why| panic!("{says:?} means {phrase:?}, which is not: {why}"));
            assert!(at > now, "{says:?} is not in the future");
        }
    }
}

/// Labels, in both directions — `plan.md` phase 7d.
///
/// `label:` has searched since F131 and sync has ingested Gmail's labels since F136, and the
/// row's own "Label" button opened nothing: `OpKind::AddLabel` has no `Op` because
/// `Op::Label` carries a payload, and `op_for` correctly returned `None` for it. Correctly,
/// and then nothing else happened.
mod naming_a_conversation {
    use super::*;

    fn a_label(store: &SqliteStore, name: &str) -> LabelId {
        let id = LabelId::generate();
        store
            .connection()
            .execute(
                "INSERT INTO labels (id, account, name, origin)
                 VALUES (?1, ?2, ?3, '\"provider\"')",
                rusqlite::params![id.to_string(), ACCOUNT.to_string(), name],
            )
            .unwrap();
        id
    }

    #[test]
    fn a_label_can_be_put_on_and_taken_off_again() {
        let (store, _dir) = realistic();
        let travel = a_label(&store, "travel");
        let thread = store
            .threads(&inbox_query(), chrono::Utc::now())
            .unwrap()
            .items[0]
            .id;

        assert!(apply_label(&store, thread, travel, Membership::In));
        assert!(
            store
                .thread(thread)
                .unwrap()
                .summary
                .labels
                .contains(&travel),
            "the label never landed"
        );

        assert!(apply_label(&store, thread, travel, Membership::Out));
        assert!(
            !store
                .thread(thread)
                .unwrap()
                .summary
                .labels
                .contains(&travel),
            "the label would not come off"
        );
    }

    #[test]
    fn labelling_a_gmail_conversation_reaches_gmail() {
        // Under `ServerLabels::Supported` a label is the server's, not ours. The fixture's
        // capabilities are the real account's, so this is the path the user's mail takes.
        let (store, _dir) = realistic();
        let travel = a_label(&store, "travel");
        let thread = store
            .threads(&inbox_query(), chrono::Utc::now())
            .unwrap()
            .items[0]
            .id;

        apply_label(&store, thread, travel, Membership::In);

        let queued = store.outbox_due(ACCOUNT, chrono::Utc::now()).unwrap();
        assert!(
            queued.iter().any(|entry| matches!(
                &entry.op,
                ProtoOp::SetLabels { add, .. } if add.iter().any(|name| name == "travel")
            )),
            "the label stopped at this machine: {:?}",
            queued.iter().map(|e| &e.op).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn the_menu_opens_from_the_row_and_lists_what_there_is() {
        dispatching();
        let (store, _dir) = realistic();
        a_label(&store, "travel");
        let mut dom = VirtualDom::new(App).with_root_context(store.clone());
        dom.rebuild_in_place();
        // The effect that fills `Shell::labels` runs on a revision; one render settles it.
        dom.render_immediate(&mut NoOpMutations);

        let page = dioxus_ssr::render(&dom);
        assert!(
            page.contains(">Label<"),
            "the rows offer no way to label anything:\n{page}"
        );
    }
}

#[test]
fn snooze_help_is_the_time_snooze_until_resolves() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
    let zone = chrono::Utc;
    let items = super::snooze_items(now, &zone);
    let expect = [
        ("later", "Later today"),
        ("tomorrow", "Tomorrow 09:00"),
        ("weekend", "This weekend"),
        ("monday", "Next week"),
    ];
    assert_eq!(items.len(), expect.len());
    for (phrase, name) in expect {
        let item = items
            .iter()
            .find(|item| item.key == phrase)
            .unwrap_or_else(|| panic!("{phrase} is not in the menu"));
        assert_eq!(item.name, name, "{phrase}");
        let at = crate::view::snooze_until(phrase, now, &zone).expect(phrase);
        assert_eq!(
            item.help.as_deref(),
            Some(super::snooze_help(at, &zone).as_str()),
            "{phrase}"
        );
    }
}

#[test]
fn create_appears_only_for_a_new_name_and_checks_follow_the_thread() {
    let travel = LabelId::generate();
    let home = LabelId::generate();
    let known = vec![("travel".to_owned(), travel), ("home".to_owned(), home)];
    let mut summary = mail_domain::ThreadSummary {
        id: ThreadId::generate(),
        account: AccountId::generate(),
        subject: "hi".to_owned(),
        snippet: String::new(),
        from: Address {
            name: None,
            email: "a@b.c".to_owned(),
        },
        participants: Vec::new(),
        recipients: Vec::new(),
        last_date: chrono::Utc::now(),
        message_count: 1,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailboxes: MailboxSet::only(MailboxRole::Inbox),
        labels: vec![travel],
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
    };
    let worn = super::label_items(&known, &summary, "");
    assert!(
        worn.iter().all(|item| !item.key.starts_with("create:")),
        "an empty filter offered to create a label: {worn:?}"
    );
    let travel_row = worn
        .iter()
        .find(|item| item.key == travel.to_string())
        .unwrap();
    let home_row = worn
        .iter()
        .find(|item| item.key == home.to_string())
        .unwrap();
    assert_eq!(travel_row.right, Right::Check(true));
    assert_eq!(home_row.right, Right::Check(false));

    let again = super::label_items(&known, &summary, "travel");
    assert!(
        again.iter().all(|item| !item.name.starts_with("Create")),
        "an existing name offered Create: {again:?}"
    );

    summary.labels.clear();
    let fresh = super::label_items(&known, &summary, "zephyr");
    let create = fresh
        .iter()
        .find(|item| item.key == "create:zephyr")
        .expect("a new name has no Create row");
    assert_eq!(create.name, "Create “zephyr”");
    assert!(
        fresh
            .iter()
            .filter(|item| item.key.starts_with("create:"))
            .count()
            == 1,
        "more than one Create row"
    );
}

/// The Labels menu open on a row, with two labels, one of them worn.
#[tokio::test]
#[ignore = "writes target/labels-menu.html and its -dark twin to look at"]
async fn render_the_labels_menu_to_a_file() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let account = built.store.thread(built.dana).unwrap().summary.account;
    for (name, worn) in [("travel", true), ("receipts", false)] {
        let id = LabelId::generate();
        built
            .store
            .connection()
            .execute(
                "INSERT INTO labels (id, account, name, origin) VALUES (?1, ?2, ?3, '\"user\"')",
                rusqlite::params![id.to_string(), account.to_string(), name],
            )
            .unwrap();
        if worn {
            apply_label(&built.store, built.dana, id, Membership::In);
        }
    }
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = crate::ui::fixtures::rebuild_into(&mut dom);
    let label = seen.all("aria-label", "Label")[0];
    crate::ui::fixtures::click(&mut dom, label);
    crate::ui::fixtures::drain(&mut dom);
    let body = dioxus_ssr::render(&dom);
    assert!(body.contains("Labels"), "the Labels menu did not open");
    crate::ui::fixtures::dump("labels-menu", &body);
}
