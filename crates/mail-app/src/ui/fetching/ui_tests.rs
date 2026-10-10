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

/// An account that must sign in again carries a mark, not a banner. The mark opens the
/// Connection Doctor; the Doctor's gear opens Settings on the account's own page, and Sign In
/// asks for the Add Account window with the address typed in. Both are other windows, so the
/// Doctor stays where it is.
#[tokio::test]
async fn a_refused_account_s_mark_opens_the_doctor_whose_gear_and_sign_in_open_their_windows() {
    let (mut dom, seen, _script, _dir) = after_a_pass(refused).await;
    let settings = Arc::new(crate::ui::settings_window::tests::Asked::default());
    dom.provide_root_context(crate::ui::settings_window::SettingsWindows(
        settings.clone(),
    ));
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

    // The mark opens the Doctor.
    let mark = seen.one("aria-label", "Sign in again to keep receiving mail.");
    let seen = click(&mut dom, mark).merge(settle_seen(&mut dom).await);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Connection Doctor"),
        "the mark: no sheet: {page}"
    );
    assert!(
        page.contains("me@nowhere.example"),
        "the mark: no account: {page}"
    );
    assert!(
        page.contains("Sign-in needed"),
        "the mark: no status: {page}"
    );
    assert!(page.contains("Check All"), "the mark: no Check All: {page}");

    // The gear asks for Settings on the account's page.
    let open = seen.one("aria-label", "Account settings for me@nowhere.example");
    click(&mut dom, open);
    settle_seen(&mut dom).await;
    assert_eq!(
        settings.asks(),
        [Some(crate::ui::settings_window::SettingsAt::Account(
            acct_account()
        ))],
        "the gear"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Check All"),
        "the gear: the doctor went: {page}"
    );

    // Sign In asks for the Add Account window, with the account's address typed in.
    let sign_in = seen.one("aria-label", "Sign In for me@nowhere.example");
    click(&mut dom, sign_in);
    settle(&mut dom).await;
    let asked = dom.in_scope(ScopeId::APP, consume_context::<super::tests::Asked>);
    assert_eq!(
        *asked.0.lock().unwrap(),
        [crate::ui::add_account::Ask {
            address: Some("me@nowhere.example".to_owned())
        }],
        "Sign In"
    );
}

#[tokio::test]
async fn the_status_line_of_a_warning_opens_the_connection_doctor() {
    let (mut dom, seen, _script, _dir) = after_a_pass(refused).await;
    let line = seen.one("data-opens", "doctor");
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

    let line = seen.one("data-opens", "doctor");
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
