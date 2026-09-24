//! The sheet as drawn: a condition refused as it is typed, Put on server through a test's
//! server, the reason where there is no server to put rules on, every class styled, and files of
//! it to look at.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

use super::RulesSheet;
use super::away_tests::{answering, own_server};
use super::list::RunProgress;
use super::server::{Pusher, reach};
use super::work::{self, Draft};
use crate::ui::data::account_rows;
use crate::ui::files::Phase;
use crate::ui::fixtures::{ACCOUNT, click, dispatching, rebuild_into, type_into};
use crate::view::Shell;

/// The sheet on `account`, alone.
#[component]
fn Sheet(account: Option<AccountId>) -> Element {
    let shell = use_signal(|| Shell {
        rules: Some(crate::view::RulesSheet { account }),
        ..Shell::default()
    });
    let revision = use_signal(|| 0u64);
    rsx! { RulesSheet { shell, revision } }
}

fn sheet(store: &Arc<SqliteStore>, account: AccountId, push: Pusher) -> VirtualDom {
    VirtualDom::new_with_props(
        Sheet,
        SheetProps {
            account: Some(account),
        },
    )
    .with_root_context(store.clone())
    .with_root_context(push)
}

/// A pusher that counts, and answers as a server that installed the script.
fn counting() -> (Pusher, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (Pusher(answering(calls.clone())), calls)
}

/// Let a push on its blocking thread land and redraw.
async fn settle(dom: &mut VirtualDom) {
    for _ in 0..20 {
        let quiet = std::time::Duration::from_millis(100);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut NoOpMutations);
    }
}

/// The opening tag of the element carrying `attribute`.
fn tag<'a>(page: &'a str, attribute: &str) -> &'a str {
    let at = page
        .find(attribute)
        .unwrap_or_else(|| panic!("no {attribute}: {page}"));
    let start = page[..at].rfind('<').unwrap();
    let end = at + page[at..].find('>').unwrap();
    &page[start..end]
}

/// Two rules on the seeded account, the second switched off.
fn two_rules(store: &SqliteStore) {
    for (name, query, actions) in [
        (
            "Bills",
            "from:bank.example subject:statement",
            vec![RuleAction::Label("Bills".to_owned()), RuleAction::MarkRead],
        ),
        (
            "Newsletters",
            "from:news@example.com -is:starred",
            vec![RuleAction::Archive],
        ),
    ] {
        let draft = Draft {
            name: name.to_owned(),
            query: query.to_owned(),
            actions,
            ..Draft::blank()
        };
        work::save(store, ACCOUNT, &draft, &Utc).unwrap();
    }
    let off = work::listed(store, ACCOUNT).unwrap().pop().unwrap().rule;
    work::switch(store, &off, RuleState::Disabled).unwrap();
}

#[tokio::test]
async fn a_condition_the_rules_cannot_read_says_why_and_save_is_refused() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::seeded();
    let (push, _) = counting();
    let mut dom = sheet(&store, ACCOUNT, push);
    let seen = rebuild_into(&mut dom);
    let opened = click(&mut dom, seen.one("aria-label", "New rule"));
    let query = opened.one("placeholder", "from:bank.example subject:statement");

    type_into(&mut dom, query, "frm:ada");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("“frm:” is not a word a rule knows."),
        "the refusal is not said: {page}"
    );
    assert!(
        tag(&page, "aria-label=\"Save New rule\"").contains("disabled"),
        "Save is offered for a condition no rule can read: {}",
        tag(&page, "aria-label=\"Save New rule\"")
    );

    // Read, it says how much it matches here now, and Save is back.
    type_into(&mut dom, query, "from:ada");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("1 conversation here matches it now."),
        "{page}"
    );
    assert!(
        !tag(&page, "aria-label=\"Save New rule\"").contains("disabled"),
        "{page}"
    );
    assert!(
        store.rules(ACCOUNT).unwrap().is_empty(),
        "typing kept a rule"
    );
}

#[tokio::test]
async fn the_list_says_each_rule_its_condition_and_what_it_does() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::seeded();
    two_rules(&store);
    let (push, _) = counting();
    let mut dom = sheet(&store, ACCOUNT, push);
    let seen = rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    let bills = page
        .find("from:bank.example subject:statement")
        .expect(&page);
    let news = page.find("from:news@example.com -is:starred").expect(&page);
    assert!(bills < news, "not in the order they run");
    assert!(page.contains("Label “Bills” · Mark read"), "{page}");
    assert!(
        tag(&page, "aria-label=\"Newsletters on\"").contains("aria-checked=\"false\""),
        "the switched-off rule is drawn on"
    );

    // Down, and the order on the page is the order in the store.
    click(&mut dom, seen.one("aria-label", "Move Bills down"));
    let page = dioxus_ssr::render(&dom);
    let bills = page.find("from:bank.example subject:statement").unwrap();
    let news = page.find("from:news@example.com -is:starred").unwrap();
    assert!(news < bills, "the move is not drawn");
    let names: Vec<String> = work::listed(&store, ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|l| l.rule.name)
        .collect();
    assert_eq!(names, ["Newsletters", "Bills"]);
}

#[tokio::test]
async fn put_on_server_shows_what_the_server_said() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    let (push, calls) = counting();
    let mut dom = sheet(&store, row.id, push);
    let seen = rebuild_into(&mut dom);
    let button = seen.one("aria-label", "Put me@nowhere.example's rules on the server");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "pushed before it was asked"
    );
    click(&mut dom, button);
    settle(&mut dom).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let page = dioxus_ssr::render(&dom);
    for said in [
        "me@nowhere.example: the server now runs 1 rule(s) and the vacation reply",
        "runs in this client only: a label exists only in this client",
    ] {
        assert!(page.contains(said), "{said:?} is not shown: {page}");
    }
}

#[tokio::test]
async fn a_provider_without_sieve_shows_the_reason_and_no_button() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let row = account_rows(&built.store)
        .into_iter()
        .find(|row| reach(&row.plan).is_err())
        .expect("the Work Space has a Google account");
    let (push, calls) = counting();
    let mut dom = sheet(&built.store, row.id, push);
    rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("offers no ManageSieve"), "{page}");
    assert!(page.contains("No vacation reply here"), "{page}");
    assert!(!page.contains("Put on server"), "a push is offered: {page}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// "Run on existing mail" in each phase it passes through.
#[component]
fn Phases() -> Element {
    rsx! {
        for phase in [
            Phase::Running { done: 200, of: 1204 },
            Phase::Finished("“Bills” looked at 1204 messages and acted on 12.".to_owned()),
            Phase::Failed("It stopped before it finished.".to_owned()),
        ] {
            RunProgress { phase, name: "Bills".to_owned() }
        }
    }
}

/// Every state the sheet draws, as one page: the list, the editor with a refusal and its menu
/// open, the vacation form, a push said, and a run in each phase.
async fn every_state(store: &Arc<SqliteStore>) -> String {
    two_rules(store);
    let row = own_server(store);
    let kept = crate::rules::server::vacation_for(
        &super::server::configured(store, &row, Utc::now()),
        "Away until October",
        "Back on the 12th.",
        Vacation::DEFAULT_DAYS,
        DateRange {
            from: None,
            to: Some(Utc::now() + chrono::TimeDelta::days(14)),
        },
    );
    store.put_vacation(row.id, Some(&kept), Utc::now()).unwrap();

    let (push, _) = counting();
    let mut rules = sheet(store, ACCOUNT, push.clone());
    let seen = rebuild_into(&mut rules);
    let editing = click(&mut rules, seen.one("aria-label", "Edit Bills"));
    type_into(
        &mut rules,
        editing.one("placeholder", "from:bank.example subject:statement"),
        "frm:bank",
    );
    click(&mut rules, editing.one("aria-expanded", "false"));

    let mut away = sheet(store, row.id, push);
    let seen = rebuild_into(&mut away);
    click(
        &mut away,
        seen.one("aria-label", "Put me@nowhere.example's rules on the server"),
    );
    settle(&mut away).await;

    let mut phases = VirtualDom::new(Phases);
    phases.rebuild_in_place();
    dioxus_ssr::render(&rules) + &dioxus_ssr::render(&away) + &dioxus_ssr::render(&phases)
}

#[tokio::test]
async fn every_class_the_rules_sheet_draws_is_styled() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::seeded();
    let markup = every_state(&store).await;
    for class in [
        "rules-row off",
        "rules-switch",
        "rules-look refused",
        "rules-action",
        "rules-menu",
        "rules-body",
        "rules-said",
        "files-bar",
    ] {
        assert!(markup.contains(class), "{class} was not drawn: {markup}");
    }
    let missing =
        crate::ui::style::tests::unstyled_classes(&markup, &crate::ui::style::tests::full_css());
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

/// `extra` as the first child of `.app`, where the window mounts its overlays.
fn inject(page: &str, extra: &str) -> String {
    let at = page.find("class=\"app").unwrap_or(0);
    let close = page[at..].find('>').map_or(page.len(), |rel| at + rel + 1);
    format!("{}{extra}{}", &page[..close], &page[close..])
}

#[tokio::test]
#[ignore = "writes target/rules*.html and their -dark twins to look at"]
async fn render_the_rules_sheet_to_files() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let mut frame = VirtualDom::new(crate::ui::app::App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    frame.rebuild_in_place();
    let backdrop = dioxus_ssr::render(&frame);
    let rows = account_rows(&built.store);
    let own = rows
        .iter()
        .find(|row| reach(&row.plan).is_ok())
        .expect("the Work Space has an account with its own server")
        .clone();
    for (name, query, actions) in [
        (
            "Rust newsletter",
            "from:newsletter@thisweekinrust.example",
            vec![RuleAction::Label("rust".to_owned()), RuleAction::MarkRead],
        ),
        (
            "Mailing list",
            "to:rust-users subject:\"[rust-users]\"",
            vec![RuleAction::File("Lists/rust-users".to_owned())],
        ),
        (
            "Build bot",
            "from:ci@example.com -subject:failed",
            vec![RuleAction::Archive],
        ),
    ] {
        let draft = Draft {
            name: name.to_owned(),
            query: query.to_owned(),
            actions,
            ..Draft::blank()
        };
        work::save(&built.store, own.id, &draft, &Utc).unwrap();
    }
    let last = work::listed(&built.store, own.id)
        .unwrap()
        .pop()
        .unwrap()
        .rule;
    work::switch(&built.store, &last, RuleState::Disabled).unwrap();
    let kept = crate::rules::server::vacation_for(
        &super::server::configured(&built.store, &own, Utc::now()),
        "Away until 12 October",
        "I am away until the 12th and reading mail when I am back.\n\nFor the release, write to release@example.com.",
        Vacation::DEFAULT_DAYS,
        DateRange {
            from: Some(Utc::now() + chrono::TimeDelta::days(3)),
            to: Some(Utc::now() + chrono::TimeDelta::days(18)),
        },
    );
    built
        .store
        .put_vacation(own.id, Some(&kept), Utc::now())
        .unwrap();

    // The list, the vacation reply and what the server said.
    let (push, _) = counting();
    let mut dom = sheet(&built.store, own.id, push.clone());
    let seen = rebuild_into(&mut dom);
    click(
        &mut dom,
        seen.one(
            "aria-label",
            &format!("Put {}'s rules on the server", own.address),
        ),
    );
    settle(&mut dom).await;
    crate::ui::fixtures::dump("rules", &inject(&backdrop, &dioxus_ssr::render(&dom)));

    // Editing a rule, with a condition it cannot read and the action menu open.
    let mut dom = sheet(&built.store, own.id, push.clone());
    let seen = rebuild_into(&mut dom);
    let editing = click(&mut dom, seen.one("aria-label", "Edit Rust newsletter"));
    type_into(
        &mut dom,
        editing.one("placeholder", "from:bank.example subject:statement"),
        "from:newsletter@thisweekinrust.example frm:weekly",
    );
    click(&mut dom, editing.one("aria-expanded", "false"));
    crate::ui::fixtures::dump("rules-edit", &inject(&backdrop, &dioxus_ssr::render(&dom)));

    // A provider with no ManageSieve: the reasons, no button.
    let google = rows.iter().find(|row| reach(&row.plan).is_err()).unwrap();
    let mut dom = sheet(&built.store, google.id, push);
    rebuild_into(&mut dom);
    crate::ui::fixtures::dump(
        "rules-provider",
        &inject(&backdrop, &dioxus_ssr::render(&dom)),
    );
}
