//! Adding an account straight in the browser: which entry goes where, the add menu, what the toast
//! says, and the main window's "+" end to end with a sign-in of the test's.

use super::direct::{
    AddPick, DirectSignIns, Entry, SignInDirectly, Signing, add_menu, entry, entry_for_plan,
    redirects, told, waiting_words,
};
use super::provider::offered_specs;
use super::{AddAccountWindows, Ask, OpenAddAccount};
use crate::ui::fixtures::{Seen, click, dispatching, empty, rebuild_into};
use crate::ui::fixtures::{menu_names, pick_named, settle as settle_menu};
use crate::ui::motion::Follow;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use ds::prelude::MenuItem;
use mail_core::account::{Mode, SignInError};
use mail_domain::AuthPlan;
use porter_provider::Issuer;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn with_clients(clients: &'static [Issuer]) -> impl Fn(Issuer) -> bool {
    move |issuer| clients.contains(&issuer)
}

/// Every provider mailo offers, with its issuer's client at hand and without: the ones that sign
/// in with a redirect go straight to the browser when they can, and every other one, or one with
/// no client, is the window.
#[test]
fn each_offered_provider_goes_where_its_sign_in_does() {
    // (provider id, where it goes with a client, where it goes without)
    const CASES: &[(&str, Entry, Entry)] = &[
        ("google", Entry::Redirect(Issuer::Google), Entry::Window),
        (
            "microsoft",
            Entry::Redirect(Issuer::Microsoft),
            Entry::Window,
        ),
        // An app password, a password or servers typed: a form, so the window.
        ("fastmail", Entry::Window, Entry::Window),
        ("icloud", Entry::Window, Entry::Window),
        ("yahoo", Entry::Window, Entry::Window),
        ("gmx", Entry::Window, Entry::Window),
        ("generic-imap", Entry::Window, Entry::Window),
    ];
    let specs = offered_specs();
    let ids: Vec<String> = specs.iter().map(|spec| spec.id.to_string()).collect();
    let listed: Vec<&str> = CASES.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(
        ids, listed,
        "every offered provider has a row here, in order"
    );
    let every = with_clients(&[Issuer::Google, Issuer::Microsoft, Issuer::Fastmail]);
    let none = with_clients(&[]);
    for (spec, (id, with, without)) in specs.iter().zip(CASES) {
        assert_eq!(entry(spec, &every), *with, "{id} with a client");
        assert_eq!(entry(spec, &none), *without, "{id} without a client");
    }
}

/// Signing an account in again follows the same rule, through the plan it was added with.
#[test]
fn an_account_signs_in_again_where_its_plan_says() {
    let now = chrono::Utc::now();
    let gmail = mail_domain::presets::preset_for_issuer(Issuer::Google, "you@gmail.com", now)
        .unwrap()
        .plan
        .auth;
    let outlook =
        mail_domain::presets::preset_for_issuer(Issuer::Microsoft, "you@outlook.com", now)
            .unwrap()
            .plan
            .auth;
    let password = AuthPlan::Password {
        username: mail_domain::Username::SameAsAddress,
        sasl: Vec::new(),
    };
    let every = with_clients(&[Issuer::Google, Issuer::Microsoft]);
    let google_only = with_clients(&[Issuer::Google]);
    // (plan, clients, where it goes)
    let cases: [(&str, &AuthPlan, &dyn Fn(Issuer) -> bool, Entry); 4] = [
        ("gmail", &gmail, &every, Entry::Redirect(Issuer::Google)),
        (
            "outlook",
            &outlook,
            &every,
            Entry::Redirect(Issuer::Microsoft),
        ),
        ("outlook, no client", &outlook, &google_only, Entry::Window),
        ("password", &password, &every, Entry::Window),
    ];
    for (name, plan, clients, expect) in cases {
        assert_eq!(entry_for_plan(plan, clients), expect, "{name}");
    }
}

/// The menu: each redirect provider by name, a rule, Other Account…; with none, Other alone.
#[test]
fn the_add_menu_names_the_redirects_then_other() {
    let offered = redirects(
        &offered_specs(),
        &with_clients(&[Issuer::Google, Issuer::Microsoft]),
    );
    let rows = add_menu(&offered);
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert_eq!(
        rows[0],
        MenuItem::new(AddPick::Redirect(Issuer::Google), "Google")
    );
    assert_eq!(
        rows[1],
        MenuItem::new(AddPick::Redirect(Issuer::Microsoft), "Microsoft")
    );
    assert_eq!(rows[2], MenuItem::Separator);
    assert_eq!(
        rows[3],
        MenuItem::new(AddPick::Other, "Other Account\u{2026}")
    );

    let alone = add_menu(&redirects(&offered_specs(), &with_clients(&[])));
    assert_eq!(
        alone,
        vec![MenuItem::new(AddPick::Other, "Other Account\u{2026}")]
    );
}

/// What the toast says at the end, and what its button offers.
#[test]
fn the_toast_says_how_it_ended_in_plain_words() {
    let google = Issuer::Google;
    let again = Mode::Again {
        address: "you@example.test".to_owned(),
    };
    let cases: Vec<(&str, Mode, Result<String, SignInError>, &str, Follow)> = vec![
        (
            "added",
            Mode::Add,
            Ok("you@example.test".to_owned()),
            "Added you@example.test",
            Follow::Nothing,
        ),
        (
            "signed in again",
            again.clone(),
            Ok("you@example.test".to_owned()),
            "Signed in to you@example.test",
            Follow::Nothing,
        ),
        (
            "closed in the browser",
            Mode::Add,
            Err(SignInError::Declined),
            "Sign-in cancelled",
            Follow::Nothing,
        ),
        (
            "cancelled here",
            Mode::Add,
            Err(SignInError::Cancelled),
            "Sign-in cancelled",
            Follow::Nothing,
        ),
        (
            "too long",
            Mode::Add,
            Err(SignInError::TimedOut),
            "The sign-in took too long. Try again when you're ready.",
            Follow::SignIn(super::Retry::Direct {
                issuer: google,
                mode: Mode::Add,
            }),
        ),
        (
            "no client",
            again.clone(),
            Err(SignInError::NoClient(google)),
            "Mail has no sign-in set up for Google.",
            Follow::SignIn(super::Retry::Window(Ask {
                address: Some("you@example.test".to_owned()),
            })),
        ),
    ];
    for (name, mode, outcome, text, follow) in cases {
        assert_eq!(
            told(google, &mode, &outcome),
            (text.to_owned(), follow),
            "{name}"
        );
    }
    assert_eq!(waiting_words(google), "Signing in with Google\u{2026}");
}

/// A direct sign-in of the test's: it records each one asked for and waits until the test ends
/// it.
#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<(Issuer, Mode)>>,
    end: Mutex<Option<tokio::sync::oneshot::Sender<Result<String, SignInError>>>>,
}

impl SignInDirectly for Fake {
    fn has_client(&self, issuer: Issuer) -> bool {
        issuer == Issuer::Google
    }

    fn sign_in(&self, issuer: Issuer, mode: Mode) -> Signing {
        self.asked.lock().unwrap().push((issuer, mode));
        let (end, ended) = tokio::sync::oneshot::channel();
        *self.end.lock().unwrap() = Some(end);
        Box::pin(async move { ended.await.unwrap_or(Err(SignInError::Cancelled)) })
    }
}

/// Keep `address` as a Google account, as the sign-in's add does (here with no sign-in to keep).
async fn keep_account(store: &mail_core::SqliteStore, address: &str) {
    let now = chrono::Utc::now();
    let preset = mail_domain::presets::preset_for_issuer(Issuer::Google, address, now).unwrap();
    mail_core::account::add_with_password(
        store,
        address,
        Some(&mail_core::account::Setup::Discovered(Box::new(preset))),
        false,
        false,
        now,
        mail_core::account::Credentials {
            password: None,
            saved: &mail_core::ClientRegistry::default(),
            secrets: &porter_secrets::MemorySecrets::default(),
            environment: &mail_core::Environment::default(),
            on_url: &|_| {},
            signed: None,
        },
    )
    .await
    .expect("the account is kept");
}

/// What asked for the add-account window.
#[derive(Clone, Default)]
struct Windows(Arc<Mutex<Vec<Ask>>>);

impl OpenAddAccount for Windows {
    fn open(&self, ask: Ask) {
        self.0.lock().unwrap().push(ask);
    }
}

/// Let what is waiting land and redraw, as the window would between frames.
async fn settle(dom: &mut VirtualDom) -> Seen {
    let mut seen = Seen::default();
    for _ in 0..12 {
        if tokio::time::timeout(Duration::from_millis(80), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    dom.render_immediate(&mut NoOpMutations);
    seen
}

#[tokio::test]
async fn google_from_the_plus_menu_signs_in_in_the_browser_with_no_window() {
    dispatching();
    let (store, _dir) = empty();
    let fake = Arc::new(Fake::default());
    let windows = Windows::default();
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store.clone())
        .with_root_context(crate::ui::revisions::Revisions::new())
        .with_root_context(DirectSignIns(fake.clone()))
        .with_root_context(AddAccountWindows(Arc::new(windows.clone())));
    let seen = rebuild_into(&mut dom);
    let seen = seen.merge(settle(&mut dom).await);

    // "+" is a menu: Google, which has a client, then Other Account….
    let plus = *seen
        .all("aria-label", "Add account")
        .last()
        .expect("the + tile");
    let opened = settle_menu(&mut dom, click(&mut dom, plus));
    let names = menu_names(&dioxus_ssr::render(&dom));
    assert_eq!(names, ["Google", "Other Account\u{2026}"]);
    pick_named(&mut dom, &opened, "Google").await;
    settle(&mut dom).await;

    // The page is asked for once, no window is, and the row says what it waits for.
    assert_eq!(*fake.asked.lock().unwrap(), [(Issuer::Google, Mode::Add)]);
    assert!(
        windows.0.lock().unwrap().is_empty(),
        "a window was asked for"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Signing in with Google"),
        "no pending row: {page}"
    );
    assert!(page.contains("Cancel sign-in"), "no way to stop it: {page}");

    // A second pick while it waits opens no second page.
    super::direct::start(Issuer::Google, Mode::Add);
    settle(&mut dom).await;
    assert_eq!(
        fake.asked.lock().unwrap().len(),
        1,
        "a second page was opened"
    );

    // The browser comes back: the account is in the store, the toast says so, the tile draws it.
    keep_account(&store, "you@example.test").await;
    let end = fake.end.lock().unwrap().take().expect("the sign-in waits");
    end.send(Ok("you@example.test".to_owned())).unwrap();
    let mut page = String::new();
    for _ in 0..10 {
        settle(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if page.contains("Added you@example.test") {
            break;
        }
    }
    assert!(page.contains("Added you@example.test"), "no toast: {page}");
    assert!(
        page.contains("aria-label=\"you@example.test\""),
        "no tile for the account: {page}"
    );
    assert!(
        !page.contains("Signing in with Google"),
        "the row stayed: {page}"
    );
    assert!(
        windows.0.lock().unwrap().is_empty(),
        "a window was asked for"
    );
}

#[tokio::test]
async fn other_account_and_a_provider_without_a_client_open_the_window() {
    dispatching();
    let (store, _dir) = empty();
    let fake = Arc::new(Fake::default());
    let windows = Windows::default();
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store)
        .with_root_context(DirectSignIns(fake.clone()))
        .with_root_context(AddAccountWindows(Arc::new(windows.clone())));
    let seen = rebuild_into(&mut dom);
    let seen = seen.merge(settle(&mut dom).await);
    let plus = *seen
        .all("aria-label", "Add account")
        .last()
        .expect("the + tile");
    let opened = settle_menu(&mut dom, click(&mut dom, plus));
    pick_named(&mut dom, &opened, "Other Account\u{2026}").await;
    settle(&mut dom).await;
    assert_eq!(
        *windows.0.lock().unwrap(),
        [Ask::default()],
        "Other Account…"
    );

    // Microsoft has no client here: no page, the window instead, which says why.
    dom.in_scope(ScopeId::APP, || {
        super::direct::start(Issuer::Microsoft, Mode::Add);
    });
    settle(&mut dom).await;
    assert!(fake.asked.lock().unwrap().is_empty(), "a page was opened");
    assert_eq!(
        windows.0.lock().unwrap().len(),
        2,
        "no window for Microsoft"
    );
}
