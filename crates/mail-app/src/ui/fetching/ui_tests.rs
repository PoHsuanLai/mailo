//! What the list says of the links behind it, in the running window: the placeholder rows while
//! the first mail comes, the banner when an account must sign in again, and the busy Sync button
//! while a pass runs. Passes are the test's own (see `tests`).

use super::tests::{
    Ending, Script, account, acct_account, finished, link, passer, refused, settle, unreachable,
    window,
};
use crate::ui::fixtures::{Seen, click};
use dioxus::prelude::{ScopeId, VirtualDom, consume_context};
use mail_core::fetch::{Link, Trigger};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// The opening tag of the element that carries `needle`.
fn tag_with<'a>(page: &'a str, needle: &str) -> &'a str {
    let at = page
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} is not on the page: {page}"));
    let start = page[..at].rfind('<').unwrap_or(0);
    let end = at + page[at..].find('>').unwrap_or(0);
    &page[start..=end]
}

#[tokio::test]
async fn a_new_account_with_no_mail_shows_placeholder_rows_and_says_why() {
    let (store, _dir) = account(false);
    let (passer, _script) = passer();
    let (mut dom, _seen) = window(store, passer);
    assert_eq!(link(&dom), Link::Fresh);
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.matches("ds-skeleton-row").count() >= 8,
        "no placeholder rows: {page}"
    );
    assert!(page.contains("Downloading your mail"), "no caption: {page}");
    assert!(
        !page.contains("Couldn\u{2019}t load"),
        "a first fetch is not a failure: {page}"
    );
}

/// [`settle`], keeping what the renders set, so that an element that appeared can be clicked.
async fn settle_seen(dom: &mut VirtualDom) -> Seen {
    let mut seen = Seen::default();
    for _ in 0..12 {
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

/// A window whose one account ended its first manual pass with `ending`, and what it drew.
async fn after_a_pass(ending: Ending) -> (VirtualDom, Seen, Script, tempfile::TempDir) {
    let (store, dir) = account(true);
    let (passer, script) = passer();
    let (mut dom, _seen) = window(store, passer);
    super::tests::fetching(&dom).sync_all(Trigger::Manual);
    settle(&mut dom).await;
    script.release.lock().unwrap().send(ending).unwrap();
    let seen = settle_seen(&mut dom).await;
    (dom, seen, script, dir)
}

#[tokio::test]
async fn an_account_that_must_sign_in_again_shows_a_mark_that_opens_the_connection_doctor() {
    let (mut dom, seen, _script, _dir) = after_a_pass(refused).await;
    assert!(matches!(link(&dom), Link::NeedsSignIn { .. }));
    let page = dioxus_ssr::render(&dom);
    // Mail puts no banner over the messages; the account carries a mark instead.
    assert!(
        !page.contains(r#"class="ds-inline-banner"#),
        "a banner is back: {page}"
    );
    assert!(!page.contains("Check All"), "open unasked: {page}");
    // Nothing can be loaded and the pane says so, with a way into the doctor.
    assert!(page.contains("Couldn\u{2019}t load this mailbox"), "{page}");
    assert!(page.contains("Connection Doctor\u{2026}"), "{page}");
    let mark = seen.one("aria-label", "Sign in again to keep receiving mail.");

    let seen = click(&mut dom, mark);
    let seen = seen.merge(settle_seen(&mut dom).await);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Connection Doctor"), "no sheet: {page}");
    assert!(page.contains("me@nowhere.example"), "no account: {page}");
    assert!(page.contains("Sign-in needed"), "no status: {page}");
    assert!(page.contains("Check All"), "no Check All: {page}");
    let sign_in = seen.one("aria-label", "Sign In for me@nowhere.example");

    // Sign In asks for the Add Account window, with the account's address typed in. The window is
    // another window: this one's doctor stays as it is.
    click(&mut dom, sign_in);
    settle(&mut dom).await;
    let asked = dom.in_scope(ScopeId::APP, consume_context::<super::tests::Asked>);
    assert_eq!(
        *asked.0.lock().unwrap(),
        [crate::ui::add_account::Ask {
            address: Some("me@nowhere.example".to_owned())
        }]
    );
}

#[tokio::test]
async fn the_status_line_of_a_warning_opens_the_connection_doctor() {
    let (mut dom, seen, _script, _dir) = after_a_pass(refused).await;
    let line = seen.one("title", "Open Connection Doctor");
    click(&mut dom, line);
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Sign-in needed"), "{page}");
    assert!(page.contains("Check All"), "no sheet: {page}");
}

#[tokio::test]
async fn try_again_in_the_connection_doctor_asks_for_a_manual_pass() {
    let (mut dom, seen, script, _dir) = after_a_pass(unreachable).await;
    assert!(matches!(link(&dom), Link::Waiting { .. }));
    assert_eq!(script.runs.load(Ordering::SeqCst), 1);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains(r#"class="ds-inline-banner"#),
        "a banner is back: {page}"
    );
    assert!(!page.contains("Check All"), "open unasked: {page}");

    let line = seen.one("title", "Open Connection Doctor");
    let seen = click(&mut dom, line);
    let seen = seen.merge(settle_seen(&mut dom).await);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Can\u{2019}t reach the server \u{2014} trying again at"),
        "{page}"
    );
    click(
        &mut dom,
        seen.one("aria-label", "Try Again for me@nowhere.example"),
    );
    settle(&mut dom).await;
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        2,
        "Try Again did not start a pass"
    );
}
#[tokio::test]
async fn the_sync_button_is_busy_while_a_pass_runs_and_not_after() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let (mut dom, seen) = window(store, passer);
    let button = r#"aria-label="Sync now""#;
    let page = dioxus_ssr::render(&dom);
    assert!(
        !tag_with(&page, button).contains(r#"aria-busy="true""#),
        "busy before anything ran"
    );

    click(&mut dom, seen.one("aria-label", "Sync now"));
    settle(&mut dom).await;
    assert_eq!(script.runs.load(Ordering::SeqCst), 1);
    let page = dioxus_ssr::render(&dom);
    assert!(
        tag_with(&page, button).contains(r#"aria-busy="true""#),
        "not busy while a pass runs: {}",
        tag_with(&page, button)
    );
    assert!(page.contains("Downloading 3 of 10"));
    assert!(page.contains("ds-progress"), "no bar for a known count");

    script.release.lock().unwrap().send(finished).unwrap();
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        !tag_with(&page, button).contains(r#"aria-busy="true""#),
        "still busy after the pass ended"
    );
}

/// The Doctor's gear opens Settings on the account's own page, and leaves the Doctor where it is.
#[tokio::test]
async fn the_doctor_s_gear_opens_settings_on_the_account_s_page() {
    let (mut dom, seen, _script, _dir) = after_a_pass(refused).await;
    let asked = Arc::new(crate::ui::settings_window::tests::Asked::default());
    dom.provide_root_context(crate::ui::settings_window::SettingsWindows(asked.clone()));
    let mark = seen.one("aria-label", "Sign in again to keep receiving mail.");
    let seen = click(&mut dom, mark).merge(settle_seen(&mut dom).await);

    let open = seen.one("aria-label", "Account settings for me@nowhere.example");
    click(&mut dom, open);
    settle_seen(&mut dom).await;
    let account = acct_account();
    assert_eq!(
        asked.asks(),
        [Some(crate::ui::settings_window::SettingsAt::Account(
            account
        ))]
    );
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Check All"), "the doctor went: {page}");
}

/// An account removed in the Settings window, which moves the shared revision, leaves this
/// window's links and its Doctor.
#[tokio::test]
async fn an_account_removed_in_settings_leaves_the_links_and_the_doctor() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let revisions = crate::ui::revisions::Revisions::new();
    crate::ui::fixtures::dispatching();
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store.clone())
        .with_root_context(passer)
        .with_root_context(revisions.clone());
    let _ = crate::ui::fixtures::rebuild_into(&mut dom);
    super::tests::fetching(&dom).sync_all(Trigger::Manual);
    settle(&mut dom).await;
    script.release.lock().unwrap().send(refused).unwrap();
    let seen = settle_seen(&mut dom).await;
    let mark = seen.one("aria-label", "Sign in again to keep receiving mail.");
    click(&mut dom, mark);
    settle_seen(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("me@nowhere.example"));

    let account = acct_account();
    let secrets = porter_secrets::MemorySecrets::default();
    mail_core::account::remove(&store, &secrets, account)
        .await
        .unwrap_or_else(|why| panic!("{why:?}"));
    let (mut settings, _) = revisions.join(0);
    revisions.publish(&mut settings);
    settle_seen(&mut dom).await;
    let links = dom.in_scope(ScopeId::APP, || super::tests::fetching(&dom).all_links());
    assert!(links.is_empty(), "its link is still running: {links:?}");
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("No accounts to check."), "{page}");
    assert!(!page.contains("me@nowhere.example"), "{page}");
}
