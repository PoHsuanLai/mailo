//! What the window says of the links behind it: the placeholder rows while the first mail
//! comes, the dialog when an account must sign in again, and the sidebar's sync mark, busy while
//! a pass runs and warning when one failed. Passes are the test's own (see `tests`).

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
async fn a_new_account_with_no_mail_shows_placeholder_rows_and_no_caption() {
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
    // The rows alone: the window opens with no words over the list.
    assert!(
        !page.contains("Downloading your mail"),
        "a caption over the rows: {page}"
    );
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

/// Open Connections the way a person reaches it now: ⌘K, its name, Return.
async fn open_connections(dom: &mut VirtualDom) -> Seen {
    use crate::ui::command::LABEL;
    use crate::ui::fixtures::{INSIDE_THE_SHELL, PRIMARY, chord, drain_seen, type_into};
    use dioxus::html::input_data::keyboard_types::Modifiers;
    chord(
        dom,
        "k",
        PRIMARY,
        dioxus::core::ElementId(INSIDE_THE_SHELL as usize),
    );
    let seen = drain_seen(dom);
    let field = *seen
        .all("aria-label", LABEL)
        .last()
        .expect("the panel's field");
    type_into(dom, field, crate::ui::doctor::TITLE);
    // The panel answers once the field is still: wait for its rows to name the sheet and to have
    // dropped Compose, the first row before it filters, or Return runs Compose instead.
    let start = std::time::Instant::now();
    loop {
        let page = dioxus_ssr::render(dom);
        let mut words = String::new();
        let mut inside = false;
        for ch in page.chars() {
            match ch {
                '<' => inside = true,
                '>' => inside = false,
                ch if !inside => words.push(ch),
                _ => {}
            }
        }
        if words.contains(crate::ui::doctor::TITLE) && !words.contains("Compose") {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the panel never answered: {page}"
        );
        settle_seen(dom).await;
    }
    chord(dom, "Enter", Modifiers::empty(), field).merge(settle_seen(dom).await)
}

/// An account that must sign in again is asked about once, in a dialog, with no banner over the
/// messages. Later puts the question off for good; Sign In, in the dialog and in the pane that has
/// nothing to list, asks for the Add Account window with the address typed in. Nothing opens
/// Connections by itself.
#[tokio::test]
async fn a_refused_account_is_asked_about_once_and_signs_in_from_the_dialog_or_the_pane() {
    let (mut dom, seen, _script, _dir) = after_a_pass(refused).await;
    assert!(matches!(link(&dom), Link::NeedsSignIn { .. }));
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains(r#"class="ds-inline-banner"#),
        "a banner is back: {page}"
    );
    assert!(
        page.contains("Sign in to me@nowhere.example again?"),
        "no dialog: {page}"
    );
    assert!(
        page.contains("can\u{2019}t fetch new mail until you do."),
        "{page}"
    );
    assert!(
        !page.contains("Check All"),
        "Connections opened unasked: {page}"
    );
    // Nothing can be loaded, and the pane says so with a Sign In of its own.
    assert!(page.contains("Couldn\u{2019}t load this mailbox"), "{page}");
    assert!(
        page.contains("Sign in to me@nowhere.example again to load its mail."),
        "{page}"
    );
    assert!(!page.contains("Connection Doctor"), "{page}");

    // Sign In asks for the Add Account window, with the account's address typed in.
    click(
        &mut dom,
        seen.one("aria-label", "Sign In to me@nowhere.example"),
    );
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains("again?"), "asked after Sign In: {page}");
    let ask = crate::ui::add_account::Ask {
        address: Some("me@nowhere.example".to_owned()),
    };
    let asked_for = |dom: &VirtualDom| {
        dom.in_scope(ScopeId::APP, consume_context::<super::tests::Asked>)
            .0
            .lock()
            .unwrap()
            .clone()
    };
    assert_eq!(
        asked_for(&dom),
        std::slice::from_ref(&ask),
        "the dialog's Sign In"
    );

    // The pane's Sign In is the same.
    click(&mut dom, seen.one("data-sign-in", "mailbox"));
    settle(&mut dom).await;
    assert_eq!(asked_for(&dom), [ask.clone(), ask], "the pane's Sign In");

    // Later, in another window, puts the question off: it is not asked again.
    let (mut dom, seen, _script, _dir) = after_a_pass(refused).await;
    click(&mut dom, seen.one("data-answer", "later"));
    settle_seen(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains("again?"), "asked after Later: {page}");
}

/// A warning is the sidebar's sync mark's: plain words in its tip and a dot on it, and no line
/// of words in the list's header.
#[tokio::test]
async fn a_warning_is_said_by_the_sync_mark_and_not_over_the_list() {
    let (dom, _seen, _script, _dir) = after_a_pass(refused).await;
    let page = dioxus_ssr::render(&dom);
    let mark = tag_with(
        &page,
        &format!("aria-label=\"{}\"", crate::ui::copy::aria::SYNC_NOW),
    );
    assert!(mark.contains("Sync \u{b7} Sign-in needed"), "{mark}");
    assert!(
        page.contains(r#"class="sync-dot""#),
        "no warning dot on the sync mark: {page}"
    );
    let head = &page[page
        .find(r#"class="list-head""#)
        .expect("the list's toolbar")..];
    let head = &head[..head.find(r#"class="list""#).unwrap_or(head.len())];
    assert!(
        !head.contains("Sign-in needed"),
        "the warning is in the list's header:\n{head}"
    );
    assert!(!page.contains("data-opens"), "it is a way in again: {page}");
}

/// Connections, reached from ⌘K, for trouble a sign-in cannot fix: its Try Again asks for a
/// manual pass, and its gear asks for Settings on the account's own page.
#[tokio::test]
async fn try_again_in_connections_asks_for_a_manual_pass_and_its_gear_opens_settings() {
    let (mut dom, _seen, script, _dir) = after_a_pass(unreachable).await;
    let settings = Arc::new(crate::ui::settings_window::tests::Asked::default());
    dom.provide_root_context(crate::ui::settings_window::SettingsWindows(
        settings.clone(),
    ));
    assert!(matches!(link(&dom), Link::Waiting { .. }));
    assert_eq!(script.runs.load(Ordering::SeqCst), 1);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains(r#"class="ds-inline-banner"#),
        "a banner is back: {page}"
    );
    assert!(
        !page.contains("again?"),
        "asked about a server that is down: {page}"
    );
    assert!(!page.contains("Check All"), "open unasked: {page}");

    let seen = open_connections(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Check All"), "no sheet: {page}");
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

    // The gear asks for Settings on the account's page, and Connections stays.
    click(
        &mut dom,
        seen.one("aria-label", "Account settings for me@nowhere.example"),
    );
    settle_seen(&mut dom).await;
    assert_eq!(
        settings.asks(),
        [Some(crate::ui::settings_window::SettingsAt::Account(
            acct_account()
        ))],
        "the gear"
    );
    assert!(dioxus_ssr::render(&dom).contains("Check All"));
}

#[tokio::test]
async fn the_sync_button_is_busy_while_a_pass_runs_and_not_after() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let (mut dom, seen) = window(store, passer);
    let button = &format!("aria-label=\"{}\"", crate::ui::copy::aria::SYNC_NOW);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !tag_with(&page, button).contains(r#"aria-busy="true""#),
        "busy before anything ran"
    );

    click(
        &mut dom,
        seen.one("aria-label", crate::ui::copy::aria::SYNC_NOW),
    );
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
/// window's links and its Connections sheet.
#[tokio::test]
async fn an_account_removed_in_settings_leaves_the_links_and_connections() {
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
    script.release.lock().unwrap().send(unreachable).unwrap();
    settle_seen(&mut dom).await;
    open_connections(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("Check All"));

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

/// Sync pressed while a pass runs starts no second one, and says so: a press that does nothing
/// visible reads as a broken button, which is what a pass stuck on a silent server looked like.
#[tokio::test]
async fn sync_while_a_pass_runs_says_it_is_already_checking() {
    let (store, _dir) = account(true);
    let (passer, script) = passer();
    let (mut dom, _seen) = window(store, passer);
    super::tests::fetching(&dom).sync_all(Trigger::Manual);
    settle(&mut dom).await;
    assert!(link(&dom).is_busy(), "no pass running: {:?}", link(&dom));
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains(super::ALREADY_SYNCING), "said unasked");

    dom.in_scope(ScopeId::APP, || {
        super::sync_now(&crate::ui::view::Shell::default());
    });
    // The toast is quire's, and its host lays it out a frame after the push.
    let mut page = String::new();
    for _ in 0..8 {
        settle(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if page.contains(super::ALREADY_SYNCING) {
            break;
        }
    }
    assert!(
        page.contains(super::ALREADY_SYNCING),
        "the press said nothing: {page}"
    );
    assert_eq!(
        script.runs.load(Ordering::SeqCst),
        1,
        "a second pass was started"
    );
}
