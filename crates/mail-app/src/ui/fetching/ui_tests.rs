//! What the list says of the links behind it, in the running window: the placeholder rows while
//! the first mail comes, the banner when an account must sign in again, and the busy Sync button
//! while a pass runs. Passes are the test's own (see `tests`).

use super::tests::{
    Ending, Script, account, finished, link, passer, refused, settle, unreachable, window,
};
use crate::ui::fixtures::{Seen, click};
use dioxus::prelude::VirtualDom;
use mail_core::fetch::{Link, Trigger};
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

    // Sign In hands the account to the Add account sheet, and the doctor steps aside.
    click(&mut dom, sign_in);
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("acct-sheet"), "no Add account sheet: {page}");
    assert!(
        !page.contains("Check All"),
        "the doctor is still over it: {page}"
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
