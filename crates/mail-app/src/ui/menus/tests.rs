use super::super::app::App;
use super::super::menu::Right;
use super::super::ops::apply_label;
use crate::ui::fixtures::{acct_account, dispatching, inbox_query, realistic};
use chrono::TimeZone;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_core::when::Clock;
use mail_core::{SqliteStore, Store};
use mail_domain::*;

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
        mail_store::testing::seed_label(
            store,
            id,
            acct_account(),
            name,
            mail_domain::LabelOrigin::Provider,
        );
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

        let queued = store
            .outbox_due(acct_account(), chrono::Utc::now())
            .unwrap();
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
        let seen = crate::ui::fixtures::rebuild_into(&mut dom);
        // The effect that fills `Shell::labels` runs on a revision; one render settles it.
        dom.render_immediate(&mut NoOpMutations);

        // A row's Label… is in its menu, and opens the labels menu where that stood.
        let first = crate::ui::fixtures::listed_subjects(&dioxus_ssr::render(&dom))
            .into_iter()
            .next()
            .expect("a row");
        crate::ui::fixtures::row_action(&mut dom, &seen, &first, "Label…").await;
        let page = dioxus_ssr::render(&dom);
        assert!(
            page.contains("aria-label=\"Labels\"") && page.contains("travel"),
            "the row's Label… opened no labels menu listing travel:\n{page}"
        );
    }
}

#[test]
fn snooze_help_is_the_time_snooze_until_resolves() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
    let zone = chrono::Utc;
    let items = super::snooze_items(now, &zone, Clock::H24);
    let expect = [
        ("later", crate::ui::copy::menus::LATER_TODAY),
        ("tomorrow", crate::ui::copy::menus::TOMORROW),
        ("weekend", crate::ui::copy::menus::THIS_WEEKEND),
        ("monday", crate::ui::copy::menus::NEXT_WEEK),
    ];
    // The four times, then Choose Time…, and not a glyph among them.
    assert_eq!(items.len(), expect.len() + 1);
    assert_eq!(
        items
            .last()
            .map(|item| (item.key.as_str(), item.name.as_str())),
        Some((super::CHOOSE, crate::ui::copy::menus::CHOOSE_TIME))
    );
    assert!(
        items
            .iter()
            .all(|item| item.tile == super::super::menu::Tile::None),
        "{items:?}"
    );
    for (phrase, name) in expect {
        let item = items
            .iter()
            .find(|item| item.key == phrase)
            .unwrap_or_else(|| panic!("{phrase} is not in the menu"));
        assert_eq!(item.name, name, "{phrase}");
        let at = mail_core::snooze::snooze_until(phrase, now, &zone).expect(phrase);
        assert_eq!(
            item.right,
            Right::Hint(super::snooze_hint(at, now, &zone, Clock::H24)),
            "{phrase}"
        );
    }
}

#[test]
fn the_snooze_menu_writes_its_hours_by_the_persons_clock() {
    // It alone wrote twelve hours, whatever the person kept; now it follows them either way.
    let now = chrono::Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
    let zone = chrono::Utc;
    let hint = |phrase: &str, clock| {
        let at = mail_core::snooze::snooze_until(phrase, now, &zone).expect(phrase);
        super::snooze_hint(at, now, &zone, clock)
    };
    assert_eq!(hint("later", Clock::H24), "18:00");
    assert_eq!(hint("later", Clock::H12), "6:00 PM");
    assert_eq!(hint("weekend", Clock::H24), "Sat 09:00");
    assert_eq!(hint("weekend", Clock::H12), "Sat 9:00 AM");
    // Past the week the day is enough, and there is no hour to write.
    let far = chrono::Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
    assert_eq!(
        super::snooze_hint(far, now, &zone, Clock::H12),
        "Mon 12 Oct"
    );
}

#[test]
fn a_typed_time_is_read_as_snooze_reads_it_and_must_be_to_come() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 9, 23, 15, 0, 0).unwrap();
    let zone = chrono::Utc;
    assert_eq!(
        super::chosen_time("+2h", now, &zone, Clock::H24),
        Ok(chrono::Utc.with_ymd_and_hms(2026, 9, 23, 17, 0, 0).unwrap())
    );
    assert_eq!(
        super::chosen_time("  ", now, &zone, Clock::H24),
        Err(crate::ui::copy::fields::TYPE_A_TIME.to_owned())
    );
    let gone = super::chosen_time("today 09:00", now, &zone, Clock::H24).unwrap_err();
    assert!(gone.contains("already passed"), "{gone}");
    assert!(super::chosen_time("whenever", now, &zone, Clock::H24).is_err());
}

#[test]
fn create_appears_only_for_a_new_name_and_checks_follow_the_thread() {
    let travel = LabelId::generate();
    let home = LabelId::generate();
    let known = vec![("travel".to_owned(), travel), ("home".to_owned(), home)];
    let mut summary = mail_domain::ThreadSummary {
        subject: "hi".to_owned(),
        from: Address {
            name: None,
            email: "a@b.c".to_owned(),
        },
        last_date: chrono::Utc::now(),
        labels: vec![travel],
        ..mail_domain::testing::summary()
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
        mail_store::testing::seed_label(
            &built.store,
            id,
            account.clone(),
            name,
            mail_domain::LabelOrigin::User,
        );
        if worn {
            apply_label(&built.store, built.dana, id, Membership::In);
        }
    }
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = crate::ui::fixtures::rebuild_into(&mut dom);
    let subject = built.store.thread(built.dana).unwrap().summary.subject;
    crate::ui::fixtures::row_action(&mut dom, &seen, &subject, "Label…").await;
    let body = dioxus_ssr::render(&dom);
    assert!(body.contains("Labels"), "the Labels menu did not open");
    crate::ui::fixtures::dump("labels-menu", &body);
}
