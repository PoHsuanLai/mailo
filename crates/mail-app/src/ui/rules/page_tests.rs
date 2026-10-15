//! The Rules page as drawn: the list of rules by name with their switches, a rule's page with a
//! condition refused as it is typed, a save that puts the rules on the server through a test's
//! server and says only a failure, the menu that moves a rule, the provider with no server to put
//! rules on, every class styled, and files of it to look at.

use ds::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use porter_core::AccountId;

use super::RulesPage;
use super::away_tests::{answering, own_server};
use super::list::RunProgress;
use super::server::{Pusher, reach};
use super::work::{self, Draft};
use crate::ui::copy;
use crate::ui::data::account_rows;
use crate::ui::files::Phase;
use crate::ui::fixtures::{
    Seen, acct_account, click, dispatching, drain_seen, rebuild_into, right_click, type_into,
};
use crate::ui::view::{RulesAt, Shell};

/// The first render, then the ones after it, keeping every attribute they set: what floats in
/// quire's overlay, and a page the stack pushes, are drawn the frames after the one that asked
/// for them, on quire's clock. Waits too for a push on its blocking thread to land.
async fn landed(dom: &mut VirtualDom, mut seen: Seen) -> Seen {
    for _ in 0..40 {
        let quiet = std::time::Duration::from_millis(150);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut seen);
    }
    seen.merge(drain_seen(dom))
}

/// The page on `account`, at `at`.
#[component]
fn Page(account: Option<AccountId>, at: RulesAt) -> Element {
    let shell = use_signal(|| Shell {
        rules: crate::ui::view::RulesPage {
            account,
            path: PanePath::new(RulesAt::List).pushed(at.clone()),
        },
        ..Shell::default()
    });
    let revision = use_signal(|| 0u64);
    crate::ui::motion::use_motion();
    // Inside a quire root, as the window has it: the menus float in its overlay, and a toast is
    // said through the window's host.
    rsx! {
        Ds {
            appearance: Appearance::default(),
            material: Material::Window,
            stylesheet: ds::assembly::ds::Inject::Host,
            RulesPage { shell, revision }
            crate::ui::motion::Toast { shell, revision }
        }
    }
}

/// The page on `account`'s list, its pushes through `push`.
fn sheet(store: &Arc<SqliteStore>, account: AccountId, push: Pusher) -> VirtualDom {
    sheet_at(store, account, push, RulesAt::List)
}

fn sheet_at(store: &Arc<SqliteStore>, account: AccountId, push: Pusher, at: RulesAt) -> VirtualDom {
    let at = match at {
        RulesAt::List => None,
        other => Some(other),
    };
    let props = PageProps {
        account: Some(account),
        at: at.unwrap_or(RulesAt::List),
    };
    VirtualDom::new_with_props(Page, props)
        .with_root_context(store.clone())
        .with_root_context(push)
}

/// A pusher that counts, and answers as a server that installed the script.
fn counting() -> (Pusher, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (Pusher(answering(calls.clone())), calls)
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

/// Two rules on `account`, the second switched off.
fn two_rules(store: &SqliteStore, account: AccountId) {
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
        work::save(store, account.clone(), &draft, &Utc).unwrap();
    }
    let off = work::listed(store, account).unwrap().pop().unwrap().rule;
    work::switch(store, &off, RuleState::Disabled).unwrap();
}

fn names(store: &SqliteStore, account: AccountId) -> Vec<String> {
    work::listed(store, account)
        .unwrap()
        .into_iter()
        .map(|listed| listed.rule.name)
        .collect()
}

#[tokio::test]
async fn a_condition_the_rules_cannot_read_says_why_and_save_is_refused() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::seeded();
    let (push, _) = counting();
    let mut dom = sheet(&store, acct_account(), push);
    let seen = {
        let first = rebuild_into(&mut dom);
        landed(&mut dom, first).await
    };
    let opened = click(&mut dom, seen.one("aria-label", copy::controls::NEW_RULE));
    let opened = landed(&mut dom, opened).await;
    let query = opened.one("aria-placeholder", "from:bank.example subject:statement");

    type_into(&mut dom, query, "frm:ada");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("“frm:” is not a word a rule knows."),
        "the refusal is not said: {page}"
    );
    let save = format!("aria-label=\"{}\"", copy::aria::SAVE_RULE);
    assert!(
        tag(&page, &save).contains("disabled"),
        "Save is offered for a condition no rule can read: {}",
        tag(&page, &save)
    );

    // Read, it says how much it matches here now, and Save is back.
    type_into(&mut dom, query, "from:ada");
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("1 conversation matches"), "{page}");
    assert!(!tag(&page, &save).contains("disabled"), "{page}");
    assert!(
        store.rules(acct_account()).unwrap().is_empty(),
        "typing kept a rule"
    );
}

#[tokio::test]
async fn saving_a_rule_puts_the_rules_on_the_server_and_goes_back_to_the_list() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    let (push, calls) = counting();
    let mut dom = sheet_at(&store, row.id.clone(), push, RulesAt::Rule(None));
    let seen = {
        let first = rebuild_into(&mut dom);
        landed(&mut dom, first).await
    };
    type_into(&mut dom, seen.one("aria-placeholder", "Bills"), "Bank");
    type_into(
        &mut dom,
        seen.one("aria-placeholder", "from:bank.example subject:statement"),
        "from:bank.example",
    );
    // An action, from the menu the Add button opens.
    let opened = click(&mut dom, seen.one("aria-expanded", "false"));
    let opened = landed(&mut dom, opened).await;
    crate::ui::fixtures::pick_named(&mut dom, &opened, "Archive").await;
    assert_eq!(calls.load(Ordering::SeqCst), 0, "pushed before a save");

    click(&mut dom, seen.one("aria-label", copy::aria::SAVE_RULE));
    landed(&mut dom, Seen::default()).await;
    assert_eq!(
        names(&store, row.id.clone()),
        ["Bank"],
        "the rule was not kept"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the save was not pushed");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(copy::controls::NEW_RULE),
        "it did not go back to the list: {page}"
    );
    // A push that worked says nothing.
    assert!(!page.contains("the server now runs"), "{page}");
}

#[tokio::test]
async fn a_push_the_server_refuses_is_said_at_the_head_of_the_page() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    two_rules(&store, row.id.clone());
    let refusing = Pusher(Arc::new(|_, _, _| Err("the server is down".to_owned())));
    let mut dom = sheet(&store, row.id.clone(), refusing);
    let seen = {
        let first = rebuild_into(&mut dom);
        landed(&mut dom, first).await
    };
    // Bills' switch: the first of the list's, in paint order.
    click(&mut dom, seen.fixed("role", "switch")[0]);
    landed(&mut dom, Seen::default()).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(&copy::errors::not_on_server("the server is down")),
        "the failure is not said: {page}"
    );
    assert_eq!(
        work::listed(&store, row.id).unwrap()[0].rule.state,
        RuleState::Disabled,
        "the switch was not kept here"
    );
}

#[tokio::test]
async fn the_list_names_each_rule_with_its_switch_and_its_menu_moves_it() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    two_rules(&store, row.id.clone());
    let (push, calls) = counting();
    let mut dom = sheet(&store, row.id.clone(), push);
    let seen = {
        let first = rebuild_into(&mut dom);
        landed(&mut dom, first).await
    };
    let page = dioxus_ssr::render(&dom);
    let bills = page.find(">Bills<").expect(&page);
    let news = page.find(">Newsletters<").expect(&page);
    assert!(bills < news, "not in the order they run");
    assert!(
        !page.contains("from:bank.example"),
        "the list says more than the name: {page}"
    );
    // A switch per rule, in the order they run: Bills on, Newsletters off.
    let switches: Vec<&str> = page
        .split('<')
        .filter(|tag| tag.contains("role=\"switch\""))
        .collect();
    assert_eq!(switches.len(), 2, "a switch per rule: {page}");
    assert!(
        switches[0].contains("aria-checked=\"true\""),
        "{}",
        switches[0]
    );
    assert!(
        switches[1].contains("aria-checked=\"false\""),
        "{}",
        switches[1]
    );

    // Right-click Bills: Move Down, and the order on the page is the order in the store.
    let bills_row = seen.one(
        "id",
        &super::row_id(&RulesAt::Rule(Some(
            work::listed(&store, row.id.clone()).unwrap()[0].rule.id,
        )))
        .unwrap(),
    );
    let opened = right_click(&mut dom, bills_row);
    let opened = landed(&mut dom, opened).await;
    let menu = crate::ui::fixtures::menu_names(&dioxus_ssr::render(&dom));
    assert_eq!(
        menu,
        [
            copy::menus::RUN_ON_EXISTING_MAIL,
            copy::menus::MOVE_DOWN,
            copy::menus::DELETE_RULE
        ],
        "the first rule cannot move up"
    );
    crate::ui::fixtures::pick_named(&mut dom, &opened, copy::menus::MOVE_DOWN).await;
    landed(&mut dom, Seen::default()).await;
    assert_eq!(names(&store, row.id), ["Newsletters", "Bills"]);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.find(">Newsletters<") < page.find(">Bills<"),
        "the move is not drawn"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the move was not pushed");
}

#[tokio::test]
async fn a_provider_without_sieve_keeps_rules_here_and_says_nothing_of_a_server() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let row = account_rows(&built.store)
        .into_iter()
        .find(|row| reach(&row.plan).is_err())
        .expect("the Work Space has a Google account");
    two_rules(&built.store, row.id.clone());
    let (push, calls) = counting();
    let mut dom = sheet(&built.store, row.id.clone(), push);
    let seen = {
        let first = rebuild_into(&mut dom);
        landed(&mut dom, first).await
    };
    click(&mut dom, seen.fixed("role", "switch")[0]);
    landed(&mut dom, Seen::default()).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a server without rules was asked"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains("server"), "a server is mentioned: {page}");

    let mut away = sheet_at(&built.store, row.id, counting().0, RulesAt::Vacation);
    {
        let first = rebuild_into(&mut away);
        landed(&mut away, first).await
    };
    let page = dioxus_ssr::render(&away);
    assert!(page.contains("No vacation reply here"), "{page}");
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

/// Every state the page draws, as one page: the list, a rule's page with a refusal and its menu
/// open, the vacation reply's page, and a run in each phase.
async fn every_state(store: &Arc<SqliteStore>) -> String {
    let row = own_server(store);
    two_rules(store, row.id.clone());
    let kept = mail_core::rules::server::vacation_for(
        &super::server::configured(store, &row, Utc::now()),
        "Away until October",
        "Back on the 12th.",
        Vacation::DEFAULT_DAYS,
        DateRange {
            from: None,
            to: Some(Utc::now() + chrono::TimeDelta::days(14)),
        },
    );
    store
        .put_vacation(row.id.clone(), Some(&kept), Utc::now())
        .unwrap();
    let (push, _) = counting();

    let mut list = sheet(store, row.id.clone(), push.clone());
    {
        let first = rebuild_into(&mut list);
        landed(&mut list, first).await
    };

    let first = work::listed(store, row.id.clone()).unwrap()[0].rule.id;
    let mut rule = sheet_at(
        store,
        row.id.clone(),
        push.clone(),
        RulesAt::Rule(Some(first)),
    );
    let seen = {
        let first = rebuild_into(&mut rule);
        landed(&mut rule, first).await
    };
    type_into(
        &mut rule,
        seen.one("aria-placeholder", "from:bank.example subject:statement"),
        "frm:bank",
    );
    click(&mut rule, seen.one("aria-expanded", "false"));
    // The action menu is quire's, drawn in the root's overlay the render after it asks.
    crate::ui::fixtures::drain(&mut rule);

    let mut away = sheet_at(store, row.id, push, RulesAt::Vacation);
    {
        let first = rebuild_into(&mut away);
        landed(&mut away, first).await
    };

    let mut phases = VirtualDom::new(Phases);
    phases.rebuild_in_place();
    dioxus_ssr::render(&list)
        + &dioxus_ssr::render(&rule)
        + &dioxus_ssr::render(&away)
        + &dioxus_ssr::render(&phases)
}

#[tokio::test]
async fn every_class_the_rules_page_draws_is_styled() {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::seeded();
    let markup = every_state(&store).await;
    for class in [
        "ds-toggle",
        "data-validity=\"invalid\"",
        "rules-action",
        "role=\"menu\"",
        "ds-field-row",
        "rules-field",
        "ds-progress",
        "ds-pane-stack",
    ] {
        assert!(markup.contains(class), "{class} was not drawn: {markup}");
    }
    let offences = crate::ui::style::tests::markup_offences(&markup);
    assert!(offences.is_empty(), "markup offences: {offences:#?}");
}

/// `extra` as the first child of `.app`, where the window mounts its overlays.
fn inject(page: &str, extra: &str) -> String {
    let at = page.find("class=\"app").unwrap_or(0);
    let close = page[at..].find('>').map_or(page.len(), |rel| at + rel + 1);
    format!("{}{extra}{}", &page[..close], &page[close..])
}

#[tokio::test]
#[ignore = "writes target/rules*.html and their -dark twins to look at"]
async fn render_the_rules_page_to_files() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let mut frame = VirtualDom::new(crate::ui::app::App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    frame.rebuild_in_place();
    let backdrop = dioxus_ssr::render(&frame);
    let own = account_rows(&built.store)
        .into_iter()
        .find(|row| reach(&row.plan).is_ok())
        .expect("the Work Space has an account with its own server");
    for (name, query, actions) in [
        (
            "Rust newsletter",
            "from:newsletter@thisweekinrust.example",
            vec![RuleAction::Label("rust".to_owned()), RuleAction::MarkRead],
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
        work::save(&built.store, own.id.clone(), &draft, &Utc).unwrap();
    }
    let (push, _) = counting();
    for (name, at) in [
        ("rules", RulesAt::List),
        ("rules-new", RulesAt::Rule(None)),
        ("rules-vacation", RulesAt::Vacation),
    ] {
        let mut dom = sheet_at(&built.store, own.id.clone(), push.clone(), at);
        {
            let first = rebuild_into(&mut dom);
            landed(&mut dom, first).await
        };
        crate::ui::fixtures::dump(name, &inject(&backdrop, &dioxus_ssr::render(&dom)));
    }
}

/// Delete Rule says so in a toast with an Undo, and the Undo puts the rule back as it was: its
/// name, its place in the order and its switch.
#[tokio::test]
async fn undo_puts_a_deleted_rule_back_as_it_was() {
    use crate::ui::motion::{Follow, Motion, Restore};
    dispatching();
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    two_rules(&store, row.id.clone());
    let before = store.rules(row.id.clone()).unwrap();
    let (push, _) = counting();
    let mut dom = sheet(&store, row.id.clone(), push);
    let seen = {
        let first = rebuild_into(&mut dom);
        landed(&mut dom, first).await
    };
    let news = work::listed(&store, row.id.clone()).unwrap()[1].rule.id;
    let news_row = seen.one("id", &super::row_id(&RulesAt::Rule(Some(news))).unwrap());
    let opened = right_click(&mut dom, news_row);
    let opened = landed(&mut dom, opened).await;
    crate::ui::fixtures::pick_named(&mut dom, &opened, copy::menus::DELETE_RULE).await;
    landed(&mut dom, Seen::default()).await;
    assert_eq!(names(&store, row.id.clone()), ["Bills"]);
    assert!(
        !dioxus_ssr::render(&dom).contains(">Newsletters<"),
        "the deleted rule is still listed"
    );
    let motion = dom.in_scope(ScopeId::APP, consume_context::<Motion>);
    let offered = motion
        .toast
        .peek()
        .clone()
        .map(|said| (said.text, said.follow));
    assert!(
        matches!(
            &offered,
            Some((text, Follow::Restore(Restore::Rule(rule))))
                if text == "Deleted the rule \u{201c}Newsletters\u{201d}" && rule.name == "Newsletters"
        ),
        "{offered:?}"
    );

    // The toast's Undo, as its button calls it.
    let toasts = motion.toasts.peek().expect("the window's toast host");
    dom.in_scope(ScopeId::APP, || toasts.on_restore.call(()));
    landed(&mut dom, Seen::default()).await;
    assert_eq!(store.rules(row.id).unwrap(), before);
    assert!(
        dioxus_ssr::render(&dom).contains(">Newsletters<"),
        "the rule is back in the store and not on the page"
    );
}
