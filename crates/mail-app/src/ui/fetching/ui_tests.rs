//! What the list says of the links behind it, in the running window: the placeholder rows while
//! the first mail comes, the banner when an account must sign in again, and the busy Sync button
//! while a pass runs. Passes are the test's own (see `tests`).

use super::tests::{account, finished, link, passer, refused, settle, window};
use crate::ui::fixtures::click;
use mail_core::fetch::{Link, Trigger};
use std::sync::atomic::Ordering;

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

#[tokio::test]
async fn an_account_that_must_sign_in_again_shows_a_banner_with_that_action() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let (mut dom, _seen) = window(store, passer);
    super::tests::fetching(&dom).sync_all(Trigger::Manual);
    settle(&mut dom).await;
    script.release.lock().unwrap().send(refused).unwrap();
    settle(&mut dom).await;
    assert!(matches!(link(&dom), Link::NeedsSignIn { .. }));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("ds-inline-banner"), "no banner: {page}");
    assert!(
        page.contains("Sign in to me@nowhere.example again"),
        "the banner does not say whom: {page}"
    );
    assert!(
        page.contains(">Sign In<") || page.contains("Sign In"),
        "no action: {page}"
    );
    // Nothing can be loaded and the pane says so, with the same account's way out above it.
    assert!(page.contains("Couldn\u{2019}t load this mailbox"), "{page}");
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
