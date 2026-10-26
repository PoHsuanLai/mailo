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
use crate::ui::view::{RulesAt, Shell, VacationAt};

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

/// The page on `account`, at `at`; Settings' Vacation Reply page instead, for `vacation`, with
/// `account`'s reply pushed over the accounts.
#[component]
fn Page(account: Option<AccountId>, at: RulesAt, #[props(default)] vacation: bool) -> Element {
    let shell = use_signal(|| Shell {
        rules: crate::ui::view::RulesPage {
            account: account.clone(),
            path: PanePath::new(RulesAt::List).pushed(at.clone()),
        },
        vacation_pane: match account.clone() {
            Some(account) if vacation => {
                PanePath::new(VacationAt::List).pushed(VacationAt::Account(account))
            }
            _ => PanePath::new(VacationAt::List),
        },
        ..Shell::default()
    });
    crate::ui::motion::use_motion();
    // Inside a quire root, as the window has it: the menus float in its overlay, and a toast is
    // said through the window's host.
    rsx! {
        Ds {
            appearance: Appearance::default(),
            material: Material::Window,
            stylesheet: ds::assembly::ds::Inject::Host,
            if vacation {
                super::VacationPage { shell }
            } else {
                RulesPage { shell }
            }
            crate::ui::motion::Toast { shell }
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
        vacation: false,
    };
    VirtualDom::new_with_props(Page, props)
        .with_root_context(store.clone())
        .with_root_context(push)
}

/// Settings' Vacation Reply page on `account`, its pushes through `push`.
fn vacation_sheet(store: &Arc<SqliteStore>, account: AccountId, push: Pusher) -> VirtualDom {
    let props = PageProps {
        account: Some(account),
        at: RulesAt::List,
        vacation: true,
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
fn two_rules(store: &Arc<SqliteStore>, account: AccountId) {
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
        work::save(
            &crate::ui::fixtures::client(store),
            account.clone(),
            &draft,
            &Utc,
        )
        .unwrap();
    }
    let off = work::listed(&crate::ui::fixtures::client(store), account)
        .unwrap()
        .pop()
        .unwrap()
        .rule;
    work::switch(
        &crate::ui::fixtures::client(store),
        &off,
        RuleState::Disabled,
    )
    .unwrap();
}

fn names(store: &Arc<SqliteStore>, account: AccountId) -> Vec<String> {
    work::listed(&crate::ui::fixtures::client(store), account)
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
    // A new rule opens on one From row, which Save waits on until it has a value.
    let page = dioxus_ssr::render(&dom);
    let save = format!("aria-label=\"{}\"", copy::aria::SAVE_RULE);
    assert!(
        tag(&page, &save).contains("disabled"),
        "{}",
        tag(&page, &save)
    );
    let text = click(
        &mut dom,
        opened.one("aria-label", copy::controls::EDIT_AS_TEXT),
    );
    let text = landed(&mut dom, text).await;
    let query = text.one("aria-placeholder", "from:bank.example subject:statement");

    type_into(&mut dom, query, "frm:ada");
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Rules don’t know “frm:”."),
        "the refusal is not said: {page}"
    );
    assert!(
        tag(&page, &save).contains("disabled"),
        "Save is offered for a condition no rule can read: {}",
        tag(&page, &save)
    );

    // Read, Save is back, nothing is said under the field, and the rows can hold it again.
    type_into(&mut dom, query, "from:ada");
    let page = dioxus_ssr::render(&dom);
    assert!(!tag(&page, &save).contains("disabled"), "{page}");
    assert!(
        !tag(
            &page,
            &format!("aria-label=\"{}\"", copy::controls::EDIT_AS_CONDITIONS)
        )
        .contains("disabled"),
        "{page}"
    );
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
    // The From row a new rule opens on.
    type_into(
        &mut dom,
        seen.one("aria-label", copy::fields::FROM),
        "bank.example",
    );
    // An action, from the menu the Add button opens.
    let opened = click(&mut dom, seen.one("aria-label", copy::aria::ADD_ACTION));
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
    assert_eq!(
        work::listed(&crate::ui::fixtures::client(&store), row.id.clone()).unwrap()[0].when,
        "from:bank.example",
        "the row was not written as the condition"
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
        work::listed(&crate::ui::fixtures::client(&store), row.id).unwrap()[0]
            .rule
            .state,
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
            work::listed(&crate::ui::fixtures::client(&store), row.id.clone()).unwrap()[0]
                .rule
                .id,
        )))
        .unwrap(),
    );
    let opened = right_click(&mut dom, bills_row);
    let opened = landed(&mut dom, opened).await;
    let menu = crate::ui::fixtures::menu_names(&dioxus_ssr::render(&dom));
    assert_eq!(
        menu,
        [copy::menus::MOVE_DOWN, copy::menus::DELETE_RULE],
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
    let row = account_rows(&crate::ui::fixtures::client(&built.store))
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
    // One line says they run only while mailo is open.
    assert!(page.contains(copy::status::RULES_RUN_HERE), "{page}");

    let mut away = vacation_sheet(&built.store, row.id, counting().0);
    {
        let first = rebuild_into(&mut away);
        landed(&mut away, first).await
    };
    // Its switch is there, greyed: there is nothing to turn on, and nothing is said.
    let page = dioxus_ssr::render(&away);
    let switch = page
        .match_indices("role=\"switch\"")
        .map(|(at, _)| {
            &page[page[..at].rfind('<').unwrap_or(0)..at + page[at..].find('>').unwrap_or(0)]
        })
        .next()
        .unwrap_or_else(|| panic!("no switch: {page}"));
    assert!(
        switch.contains("disabled"),
        "the switch can be turned on: {switch}"
    );
    assert!(!page.contains("No vacation reply here"), "{page}");
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
            OnePhase { phase }
        }
    }
}

#[component]
fn OnePhase(phase: Phase) -> Element {
    let phase = use_signal(|| phase);
    rsx! { RunProgress { phase, name: "Bills".to_owned() } }
}

/// Every state the page draws, as one page: the list, a rule's page with a refusal and its menu
/// open, the vacation reply's page, and a run in each phase.
async fn every_state(store: &Arc<SqliteStore>) -> String {
    let row = own_server(store);
    two_rules(store, row.id.clone());
    let kept = mail_core::rules::server::vacation_for(
        &super::server::configured(&crate::ui::fixtures::client(store), &row, Utc::now()),
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

    let first = work::listed(&crate::ui::fixtures::client(store), row.id.clone()).unwrap()[0]
        .rule
        .id;
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
    // Bills' condition as text, given a word no rule knows.
    let text = click(
        &mut rule,
        seen.one("aria-label", copy::controls::EDIT_AS_TEXT),
    );
    let text = landed(&mut rule, text).await;
    type_into(
        &mut rule,
        text.one("aria-placeholder", "from:bank.example subject:statement"),
        "frm:bank",
    );
    click(&mut rule, seen.one("aria-label", copy::aria::ADD_ACTION));
    // The action menu is quire's, drawn in the root's overlay the render after it asks.
    crate::ui::fixtures::drain(&mut rule);

    // And a new rule's rows, to draw them.
    let mut rows = sheet_at(store, row.id.clone(), push.clone(), RulesAt::Rule(None));
    {
        let first = rebuild_into(&mut rows);
        landed(&mut rows, first).await
    };

    let mut away = vacation_sheet(store, row.id, push);
    {
        let first = rebuild_into(&mut away);
        landed(&mut away, first).await
    };

    let mut phases = VirtualDom::new(Phases);
    phases.rebuild_in_place();
    dioxus_ssr::render(&list)
        + &dioxus_ssr::render(&rule)
        + &dioxus_ssr::render(&rows)
        + &dioxus_ssr::render(&away)
        + &dioxus_ssr::render(&phases)
}

/// Every state the page draws, on a seeded store, for the style lint in `style::tests`.
pub(in crate::ui) async fn drawn_states() -> String {
    dispatching();
    let (store, _dir) = crate::ui::fixtures::seeded();
    let markup = every_state(&store).await;
    for class in [
        "ds-toggle",
        "data-validity=\"invalid\"",
        "rules-line",
        "rules-end",
        "rules-value",
        "role=\"menu\"",
        "ds-field-row",
        "rules-field",
        "ds-progress",
        "ds-pane-stack",
    ] {
        assert!(markup.contains(class), "{class} was not drawn: {markup}");
    }
    markup
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
    let news = work::listed(&crate::ui::fixtures::client(&store), row.id.clone()).unwrap()[1]
        .rule
        .id;
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
