//! An account's page in Settings' Accounts pane, over the reference fixture: its row pushes it
//! under a back button to Accounts, the back button and Escape go back to the list, Remove
//! Account… asks before it removes and then goes back, and the window opens on the page when the
//! main window asks for it.

use super::Seams;
use crate::ui::data::account_rows;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, Work, click, dispatching, drain_seen, press, rebuild_into, work,
};
use crate::ui::settings_window::tests::opened_on;
use crate::ui::settings_window::{SettingsAsked, SettingsAt, settings_root};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::VirtualDom;
use mail_core::Storage;
use porter_core::AccountId;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The account the tests open.
const ADDRESS: &str = "poh@acme.example";

fn poh(built: &Work) -> AccountId {
    account_rows(&crate::ui::fixtures::client(&built.store))
        .into_iter()
        .find(|row| row.address == ADDRESS)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("the fixture has no {ADDRESS}"))
}

/// The Settings window on Accounts with `ADDRESS`'s page pushed by its row.
pub(in crate::ui) fn pushed(built: &Work) -> (VirtualDom, Seen) {
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

/// Render until `done` holds of the page, bounded by the wall clock: the removal and the list's
/// refresh each land from a task, so on a slow runner they can come after any fixed number of
/// quiet frames. Waits for the state, not for quiet; the page comes back for the failure message.
async fn until(dom: &mut VirtualDom, done: impl Fn(&str) -> bool) -> (bool, String) {
    let started = std::time::Instant::now();
    loop {
        let page = dioxus_ssr::render(dom);
        if done(&page) {
            return (true, page);
        }
        if started.elapsed() > Duration::from_secs(30) {
            return (false, page);
        }
        if tokio::time::timeout(Duration::from_millis(80), dom.wait_for_work())
            .await
            .is_ok()
        {
            dom.render_immediate(&mut Seen::default());
        }
    }
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
    assert!(
        page.contains(crate::ui::copy::controls::ADD_ACCOUNT),
        "{page}"
    );
    // The list names each account by its address and its provider's mark, never its protocol.
    // The page's words: its text, not its stylesheets or attributes.
    let words: String = page
        .split("</style>")
        .map(|part| part.split("<style").next().unwrap_or_default())
        .collect::<String>()
        .split('<')
        .filter_map(|tag| tag.split_once('>').map(|(_, text)| text))
        .collect::<Vec<_>>()
        .join(" ");
    for protocol in ["IMAP", "JMAP", "POP3", "SMTP", "Graph"] {
        assert!(
            !words.contains(protocol),
            "the list says {protocol}: {words}"
        );
    }

    let (dom, _) = pushed(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(on_account_page(&page), "no back button: {page}");
    assert!(
        page.contains(&format!(r#"<h2 class="ds-page-title">{ADDRESS}</h2>"#)),
        "the page is not titled with the account: {page}"
    );
    assert!(page.contains("Receiving"), "{page}");
    assert!(page.contains("Signs in"), "{page}");
    assert!(page.contains("data-page=\"Accounts\""), "{page}");
    // Download All Mail, the servers, Encryption, and Remove Account… last, alone.
    let mut from = 0;
    for drawn in [
        crate::ui::copy::fields::DOWNLOAD_ALL_MAIL.to_owned(),
        "Receiving".to_owned(),
        crate::ui::copy::headers::ENCRYPTION.to_owned(),
        format!(r#"aria-label="Remove {ADDRESS}""#),
    ] {
        from += page[from..]
            .find(&drawn)
            .unwrap_or_else(|| panic!("no {drawn:?} after what comes before it: {page}"));
    }
    assert!(
        !page[from..].contains("ds-form-section"),
        "a section comes after Remove Account…: {page}"
    );
}

#[tokio::test]
async fn the_back_button_goes_back_to_the_list() {
    let built = work();
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
    let before = account_rows(&crate::ui::fixtures::client(&built.store)).len();
    let (mut dom, seen) = pushed(&built);
    let ask = seen.one("aria-label", &format!("Remove {ADDRESS}"));
    let asked = click(&mut dom, ask).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(&format!("Remove {ADDRESS}?")), "{page}");
    assert!(page.contains("Mail on the server isn’t deleted."), "{page}");
    assert_eq!(
        account_rows(&crate::ui::fixtures::client(&built.store)).len(),
        before,
        "removed unasked"
    );

    // Escape while the question is up stays on the page.
    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let _ = drain_seen(&mut dom);
    assert!(on_account_page(&dioxus_ssr::render(&dom)));

    click(&mut dom, alert_confirm(&asked));
    let gone = format!("Details for {ADDRESS}");
    let (listed_without, page) = until(&mut dom, |page| {
        !on_account_page(page) && !page.contains(&gone)
    })
    .await;
    let left = account_rows(&crate::ui::fixtures::client(&built.store));
    assert_eq!(left.len(), before - 1);
    assert!(
        left.iter().all(|row| row.address != ADDRESS),
        "{ADDRESS} is still stored"
    );
    assert!(!on_account_page(&page), "still on the page: {page}");
    assert!(listed_without, "the list still has it: {page}");
}

#[tokio::test]
async fn the_window_opens_on_the_account_s_page_the_main_window_asks_for() {
    let built = work();
    let asked = SettingsAsked::default();
    asked.ask(SettingsAt::Account(poh(&built)));
    crate::ui::fixtures::dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(crate::ui::fixtures::client(&built.store))
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
    let (turned, page) = until(&mut dom, on_account_page).await;
    assert!(turned, "it did not turn: {page}");
}

#[tokio::test]
async fn the_page_says_how_much_storage_the_server_counts_once_it_has_said() {
    let built = work();
    let asked: Arc<Mutex<Vec<AccountId>>> = Arc::default();
    let seams = Seams {
        secrets: Arc::new(mail_core::NoKeyring),
        storage: Arc::new({
            let asked = asked.clone();
            move |_, account| {
                asked.lock().unwrap().push(account);
                Ok(Some(Storage {
                    used: 3 << 30,
                    limit: 15 << 30,
                }))
            }
        }),
    };
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone())
        .with_root_context(seams);
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let seen = click(
        &mut dom,
        seen.one("aria-label", SettingsPage::Accounts.name()),
    )
    .merge(drain_seen(&mut dom));
    // The list asks no server: only an account's own page does.
    assert!(asked.lock().unwrap().is_empty());
    let row = seen.one("aria-label", &format!("Details for {ADDRESS}"));
    let _ = click(&mut dom, row).merge(drain_seen(&mut dom));
    let storage = crate::ui::copy::fields::account::STORAGE;
    let (shown, page) = until(&mut dom, |page| page.contains(storage)).await;
    assert!(shown, "no storage row: {page}");
    assert_eq!(*asked.lock().unwrap(), vec![poh(&built)]);
    // Under the servers, before the keys.
    let at = |drawn: &str| {
        page.find(drawn)
            .unwrap_or_else(|| panic!("no {drawn}: {page}"))
    };
    assert!(at(crate::ui::copy::fields::account::RECEIVING) < at(storage));
    assert!(at(storage) < at(crate::ui::copy::headers::ENCRYPTION));
}

/// An account that waits for a sign-in is not asked for its storage: the page would only try the
/// sign-in again.
#[tokio::test]
async fn an_account_waiting_to_sign_in_is_not_asked_for_its_storage() {
    use crate::ui::fetching::{Need, Needing};
    let built = work();
    let asked: Arc<Mutex<Vec<AccountId>>> = Arc::default();
    let seams = Seams {
        secrets: Arc::new(mail_core::NoKeyring),
        storage: Arc::new({
            let asked = asked.clone();
            move |_, account| {
                asked.lock().unwrap().push(account);
                Ok(None)
            }
        }),
    };
    let needing = Needing::default();
    needing.say(vec![(poh(&built), Need::SignIn)]);
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone())
        .with_root_context(seams)
        .with_root_context(needing);
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let seen = click(
        &mut dom,
        seen.one("aria-label", SettingsPage::Accounts.name()),
    )
    .merge(drain_seen(&mut dom));
    let row = seen.one("aria-label", &format!("Details for {ADDRESS}"));
    let _ = click(&mut dom, row).merge(drain_seen(&mut dom));
    let _ = until(&mut dom, on_account_page).await;
    assert!(
        asked.lock().unwrap().is_empty(),
        "asked a server that needs a sign-in"
    );
}

#[tokio::test]
async fn without_a_server_to_ask_the_page_says_nothing_of_storage() {
    let built = work();
    let (mut dom, _) = pushed(&built);
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(on_account_page(&page), "{page}");
    assert!(
        !page.contains(crate::ui::copy::fields::account::STORAGE),
        "{page}"
    );
}
