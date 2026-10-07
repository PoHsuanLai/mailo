//! An account's page in Settings' Accounts pane, over the reference fixture: its row pushes it
//! under a back button to Accounts, the back button and Escape go back to the list, Remove
//! Account… asks before it removes and then goes back, and the window opens on the page when the
//! main window asks for it.

use crate::ui::data::account_rows;
use crate::ui::fixtures::{INSIDE_THE_SHELL, Seen, Work, click, drain_seen, press, work};
use crate::ui::settings_window::tests::opened_on;
use crate::ui::settings_window::{SettingsAsked, SettingsAt, settings_root};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::VirtualDom;
use porter_core::AccountId;
use std::time::Duration;

/// The account the tests open.
const ADDRESS: &str = "poh@acme.example";

fn poh(built: &Work) -> AccountId {
    account_rows(&built.store)
        .into_iter()
        .find(|row| row.address == ADDRESS)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("the fixture has no {ADDRESS}"))
}

/// The Settings window on Accounts with `ADDRESS`'s page pushed by its row.
fn pushed(built: &Work) -> (VirtualDom, Seen) {
    let (mut dom, seen) = opened_on(built, SettingsPage::Accounts);
    let row = seen.one("aria-label", &format!("Details for {ADDRESS}"));
    let seen = seen.merge(click(&mut dom, row)).merge(drain_seen(&mut dom));
    (dom, seen)
}

/// Whether the pane shows an account's page, the page a slide arrives at: the stack's page that
/// is not leaving holds the back button, which names the list. A leaving page stays drawn until
/// its slide rests, which a test's VirtualDom never plays.
fn on_account_page(page: &str) -> bool {
    page.split(r#"class="ds-pane-stack-page""#)
        .skip(1)
        .find(|drawn| !drawn[..drawn.find('>').unwrap_or(0)].contains(r#"data-presence="leaving""#))
        .is_some_and(|drawn| drawn.contains(r#"aria-label="Back to Accounts""#))
}

/// Let the removal's task land and redraw, keeping what the renders set.
async fn settle(dom: &mut VirtualDom) -> Seen {
    let mut seen = Seen::default();
    for _ in 0..20 {
        if tokio::time::timeout(Duration::from_millis(80), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut seen);
    }
    seen
}

/// The alert's destructive button: the last destructive control drawn, after the page's own.
fn alert_confirm(seen: &Seen) -> dioxus::dioxus_core::ElementId {
    *seen
        .all("data-role", "destructive")
        .last()
        .expect("the alert has a destructive button")
}

#[tokio::test]
async fn an_account_s_row_pushes_its_page_under_a_back_button_to_accounts() {
    let built = work();
    let (dom, _) = opened_on(&built, SettingsPage::Accounts);
    let page = dioxus_ssr::render(&dom);
    assert!(!on_account_page(&page), "pushed before a click: {page}");
    assert!(page.contains("Add Account\u{2026}"), "{page}");

    let (dom, _) = pushed(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(on_account_page(&page), "no back button: {page}");
    assert!(
        page.contains(&format!(r#"<h2 class="ds-page-title">{ADDRESS}</h2>"#)),
        "the page is not titled with the account: {page}"
    );
    assert!(page.contains("Receiving"), "{page}");
    assert!(page.contains("Signs in"), "{page}");
    assert!(
        page.contains(&format!(r#"aria-label="Remove {ADDRESS}""#)),
        "{page}"
    );
    assert!(page.contains("data-page=\"Accounts\""), "{page}");
}

#[tokio::test]
async fn escape_and_the_back_button_go_back_to_the_list() {
    let built = work();
    let (mut dom, _) = pushed(&built);
    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(!on_account_page(&page), "Escape left the page: {page}");
    assert!(page.contains("data-page=\"Accounts\""), "{page}");

    let (mut dom, seen) = pushed(&built);
    click(&mut dom, seen.one("aria-label", "Back to Accounts"));
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !on_account_page(&page),
        "the back button left the page: {page}"
    );
}

#[tokio::test]
async fn remove_asks_then_removes_the_account_and_goes_back_to_the_list() {
    let built = work();
    let before = account_rows(&built.store).len();
    let (mut dom, seen) = pushed(&built);
    let ask = seen.one("aria-label", &format!("Remove {ADDRESS}"));
    let asked = click(&mut dom, ask).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(&format!("Remove {ADDRESS}?")), "{page}");
    assert!(
        page.contains("Mail on the server is not touched."),
        "{page}"
    );
    assert_eq!(account_rows(&built.store).len(), before, "removed unasked");

    // Escape while the question is up stays on the page.
    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let _ = drain_seen(&mut dom);
    assert!(on_account_page(&dioxus_ssr::render(&dom)));

    click(&mut dom, alert_confirm(&asked));
    settle(&mut dom).await;
    let left = account_rows(&built.store);
    assert_eq!(left.len(), before - 1);
    assert!(
        left.iter().all(|row| row.address != ADDRESS),
        "{ADDRESS} is still stored"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(!on_account_page(&page), "still on the page: {page}");
    assert!(
        !page.contains(&format!("Details for {ADDRESS}")),
        "the list still has it: {page}"
    );
}

#[tokio::test]
async fn the_window_opens_on_the_account_s_page_the_main_window_asks_for() {
    let built = work();
    let asked = SettingsAsked::default();
    asked.ask(SettingsAt::Account(poh(&built)));
    crate::ui::fixtures::dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(asked.clone());
    let _ = crate::ui::fixtures::rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("data-page=\"Accounts\""), "{page}");
    assert!(on_account_page(&page), "not pushed: {page}");
    assert!(page.contains(ADDRESS), "{page}");

    // Asked again while open, after going back.
    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let _ = drain_seen(&mut dom);
    assert!(!on_account_page(&dioxus_ssr::render(&dom)));
    asked.ask(SettingsAt::Account(poh(&built)));
    settle(&mut dom).await;
    assert!(
        on_account_page(&dioxus_ssr::render(&dom)),
        "it did not turn"
    );
}
