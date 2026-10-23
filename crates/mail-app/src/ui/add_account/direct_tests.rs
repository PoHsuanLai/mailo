//! Adding an account straight in the browser: which entry goes where, the add menu, what the toast
//! says, and the Space menu's Add Account end to end with a sign-in of the test's.

use super::direct::{
    AddPick, DirectSignIns, Entry, SignInDirectly, Signing, add_menu, entry, entry_for_plan,
    offered_redirects, picked, redirects, told,
};
use super::provider::offered_specs;
use super::{AddAccountWindows, Ask, Join, OpenAddAccount};
use crate::ui::fixtures::settle_quiet_seen;
use crate::ui::fixtures::{dispatching, empty, rebuild_into};
use crate::ui::motion::Follow;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use ds::prelude::MenuItem;
use mail_core::account::{Mode, SignInError};
use mail_core::{ClientRegistry, Environment};
use mail_domain::AuthPlan;
use porter_provider::Issuer;
use std::sync::{Arc, Mutex};

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
    type Case<'a> = (&'a str, &'a AuthPlan, &'a dyn Fn(Issuer) -> bool, Entry);
    let cases: [Case; 4] = [
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
    type Case<'a> = (
        &'a str,
        Mode,
        Result<String, SignInError>,
        Option<(&'a str, Follow)>,
    );
    let cases: Vec<Case> = vec![
        (
            "added",
            Mode::Add,
            Ok("you@example.test".to_owned()),
            Some(("Added you@example.test", Follow::Nothing)),
        ),
        (
            "signed in again",
            again.clone(),
            Ok("you@example.test".to_owned()),
            Some(("Signed in to you@example.test", Follow::Nothing)),
        ),
        (
            "closed in the browser",
            Mode::Add,
            Err(SignInError::Declined),
            Some(("Sign-in cancelled", Follow::Nothing)),
        ),
        // Replaced by another pick, or never heard back from: nothing was on screen waiting.
        ("replaced", Mode::Add, Err(SignInError::Cancelled), None),
        ("too long", Mode::Add, Err(SignInError::TimedOut), None),
        (
            "no client",
            again.clone(),
            Err(SignInError::NoClient(google)),
            Some((
                "Mail has no sign-in set up for Google.",
                Follow::SignIn(super::Retry::Window(Ask {
                    address: Some("you@example.test".to_owned()),
                    join: Join::Nowhere,
                })),
            )),
        ),
    ];
    for (name, mode, outcome, want) in cases {
        assert_eq!(
            told(google, &mode, Join::OnScreen, &outcome),
            want.map(|(text, follow)| (text.to_owned(), follow)),
            "{name}"
        );
    }
}

/// A direct sign-in of the test's: it has the clients it is given, records each sign-in asked
/// for and waits until the test ends it.
struct Fake {
    clients: ClientRegistry,
    asked: Mutex<Vec<(Issuer, Mode)>>,
    end: Mutex<Option<tokio::sync::oneshot::Sender<Result<String, SignInError>>>>,
}

impl Fake {
    fn with(clients: ClientRegistry) -> Fake {
        Fake {
            clients,
            asked: Mutex::default(),
            end: Mutex::default(),
        }
    }
}

impl SignInDirectly for Fake {
    fn has_client(&self, issuer: Issuer) -> bool {
        mail_core::account::oauth_client(issuer, &Environment::default(), &self.clients).is_some()
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
            saved: &ClientRegistry::default(),
            secrets: &porter_secrets::MemorySecrets::default(),
            environment: &Environment::default(),
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

/// The Space menu's Add Account, as the window offers it now: its rows' words and what each
/// asks for.
fn add_rows(dom: &VirtualDom) -> Vec<(String, AddPick)> {
    let offered = dom.in_scope(ScopeId::APP, offered_redirects);
    add_menu(&offered)
        .into_iter()
        .filter_map(|row| match row {
            MenuItem::Item { title, value, .. } => Some((title, value)),
            _ => None,
        })
        .collect()
}

/// Pick the Add Account row named `name`, as the Space's menu does.
fn pick_add(dom: &VirtualDom, name: &str) {
    let rows = add_rows(dom);
    let pick = rows
        .iter()
        .find(|(title, _)| title == name)
        .map(|(_, pick)| *pick)
        .unwrap_or_else(|| panic!("no Add Account row is named {name:?}: {rows:?}"));
    dom.in_scope(ScopeId::APP, || picked(pick, Join::OnScreen));
}

fn names(rows: &[(String, AddPick)]) -> Vec<&str> {
    rows.iter().map(|(title, _)| title.as_str()).collect()
}

#[tokio::test]
async fn google_from_the_space_menu_signs_in_in_the_browser_with_no_window() {
    dispatching();
    let (store, _dir) = empty();
    let fake = Arc::new(Fake::with(mail_core::testing::fake_clients_for(&[
        Issuer::Google,
    ])));
    let windows = Windows::default();
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store.clone())
        .with_root_context(crate::ui::revisions::Revisions::new())
        .with_root_context(DirectSignIns(fake.clone()))
        .with_root_context(AddAccountWindows(Arc::new(windows.clone())));
    let _ = rebuild_into(&mut dom);
    settle_quiet_seen(&mut dom).await;

    // Add Account is a menu: Google, which has a client, then Other Account….
    assert_eq!(names(&add_rows(&dom)), ["Google", "Other Account\u{2026}"]);
    pick_add(&dom, "Google");
    settle_quiet_seen(&mut dom).await;

    // The page is asked for once and no window is; nothing is drawn while it waits.
    assert_eq!(*fake.asked.lock().unwrap(), [(Issuer::Google, Mode::Add)]);
    assert!(
        windows.0.lock().unwrap().is_empty(),
        "a window was asked for"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("Signing in with"),
        "a row waits for the browser: {page}"
    );

    // A second pick while it waits (the first page never came back) starts it afresh, and the
    // one it replaced ends without a word.
    dom.in_scope(ScopeId::APP, || {
        super::direct::start(Issuer::Google, Mode::Add, Join::OnScreen)
    });
    settle_quiet_seen(&mut dom).await;
    assert_eq!(
        *fake.asked.lock().unwrap(),
        [(Issuer::Google, Mode::Add), (Issuer::Google, Mode::Add)],
        "the page was not opened again"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("Sign-in cancelled"),
        "the replaced sign-in said so: {page}"
    );

    // The browser comes back: the account is in the store and the toast says so.
    keep_account(&store, "you@example.test").await;
    let end = fake.end.lock().unwrap().take().expect("the sign-in waits");
    end.send(Ok("you@example.test".to_owned())).unwrap();
    let mut page = String::new();
    for _ in 0..10 {
        settle_quiet_seen(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if page.contains("Added you@example.test") {
            break;
        }
    }
    assert!(page.contains("Added you@example.test"), "no toast: {page}");
    assert!(
        windows.0.lock().unwrap().is_empty(),
        "a window was asked for"
    );
}

/// The page once a Google account added in the browser, joining the Space `join` names, is told.
async fn added_joining(join: Join) -> String {
    dispatching();
    let (store, _dir) = empty();
    let fake = Arc::new(Fake::with(mail_core::testing::fake_clients_for(&[
        Issuer::Google,
    ])));
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store.clone())
        .with_root_context(crate::ui::revisions::Revisions::new())
        .with_root_context(DirectSignIns(fake.clone()))
        .with_root_context(AddAccountWindows(Arc::new(Windows::default())));
    let _ = rebuild_into(&mut dom);
    settle_quiet_seen(&mut dom).await;
    dom.in_scope(ScopeId::APP, || {
        super::direct::start(Issuer::Google, Mode::Add, join)
    });
    settle_quiet_seen(&mut dom).await;
    keep_account(&store, "you@example.test").await;
    let end = fake.end.lock().unwrap().take().expect("the sign-in waits");
    end.send(Ok("you@example.test".to_owned())).unwrap();
    let mut page = String::new();
    for _ in 0..10 {
        settle_quiet_seen(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if page.contains("Added you@example.test") {
            break;
        }
    }
    assert!(page.contains("Added you@example.test"), "no toast: {page}");
    page
}

/// Accounts and Spaces are apart: an account added from a Space joins it, and one added from
/// Settings › Accounts joins no Space, though it is added all the same.
#[tokio::test]
async fn an_account_added_joins_the_space_it_was_added_from_or_none() {
    let tile = "aria-label=\"you@example.test\"";
    let here = added_joining(Join::OnScreen).await;
    assert!(here.contains(tile), "not in the Space on screen: {here}");
    let nowhere = added_joining(Join::Nowhere).await;
    assert!(!nowhere.contains(tile), "put in a Space: {nowhere}");
}

#[tokio::test]
async fn other_account_and_a_provider_without_a_client_open_the_window() {
    dispatching();
    let (store, _dir) = empty();
    let fake = Arc::new(Fake::with(mail_core::testing::fake_clients_for(&[
        Issuer::Google,
    ])));
    let windows = Windows::default();
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store)
        .with_root_context(DirectSignIns(fake.clone()))
        .with_root_context(AddAccountWindows(Arc::new(windows.clone())));
    let _ = rebuild_into(&mut dom);
    settle_quiet_seen(&mut dom).await;
    pick_add(&dom, "Other Account\u{2026}");
    settle_quiet_seen(&mut dom).await;
    assert_eq!(
        *windows.0.lock().unwrap(),
        [Ask::default()],
        "Other Account…"
    );

    // Microsoft has no client here: no page, the window instead, which says why.
    dom.in_scope(ScopeId::APP, || {
        super::direct::start(Issuer::Microsoft, Mode::Add, Join::OnScreen);
    });
    settle_quiet_seen(&mut dom).await;
    assert!(fake.asked.lock().unwrap().is_empty(), "a page was opened");
    assert_eq!(
        windows.0.lock().unwrap().len(),
        2,
        "no window for Microsoft"
    );
}

#[tokio::test]
async fn with_clients_at_hand_add_account_names_google_and_microsoft() {
    dispatching();
    let (store, _dir) = empty();
    // Both stand-ins at hand, as an installation with both clients has: each is named, and a pick goes
    // to the sign-in (here the test's recorder), never to the window.
    let fake = Arc::new(Fake::with(mail_core::testing::fake_clients()));
    let windows = Windows::default();
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store)
        .with_root_context(crate::ui::revisions::Revisions::new())
        .with_root_context(DirectSignIns(fake.clone()))
        .with_root_context(AddAccountWindows(Arc::new(windows.clone())));
    let _ = rebuild_into(&mut dom);
    settle_quiet_seen(&mut dom).await;
    assert_eq!(
        names(&add_rows(&dom)),
        ["Google", "Microsoft", "Other Account\u{2026}"]
    );
    pick_add(&dom, "Microsoft");
    settle_quiet_seen(&mut dom).await;
    assert_eq!(
        *fake.asked.lock().unwrap(),
        [(Issuer::Microsoft, Mode::Add)]
    );
    assert!(
        windows.0.lock().unwrap().is_empty(),
        "a window was asked for"
    );
}

/// A window whose root was not handed the program's sign-in (every test's, unless it hands a
/// recorder) offers no provider by name and has nothing to sign in with: Add Account and a sign-in again
/// open the add-account window, and nothing reaches a browser or an issuer. Nor has the edge any
/// client, since only `main` installs them.
#[tokio::test]
async fn a_root_without_the_programs_sign_in_offers_no_redirect_and_reaches_nothing() {
    dispatching();
    assert_eq!(
        crate::edge::clients().load().unwrap(),
        ClientRegistry::default(),
        "the edge has clients no test installed"
    );
    // (root, whether it hands a recorder for the add-account window)
    const CASES: &[(&str, bool)] = &[
        ("the main window, its store alone", false),
        ("the main window, with a recorder for the window", true),
    ];
    for (name, recorded) in CASES {
        let (store, _dir) = empty();
        let windows = Windows::default();
        let mut dom = VirtualDom::new(crate::ui::app::App)
            .with_root_context(store)
            .with_root_context(crate::ui::revisions::Revisions::new());
        if *recorded {
            dom = dom.with_root_context(AddAccountWindows(Arc::new(windows.clone())));
        }
        let _ = rebuild_into(&mut dom);
        settle_quiet_seen(&mut dom).await;
        let offered = dom.in_scope(ScopeId::APP, offered_redirects);
        assert!(offered.is_empty(), "{name}: offered {offered:?}");
        // The keys' seams too: no lookup and no keyring of the user's.
        let looked = dom.in_scope(ScopeId::APP, || {
            let seams = crate::ui::pgp::seams();
            let store = consume_context::<Arc<mail_core::SqliteStore>>();
            (seams.lookup)(store.as_ref(), "ada@example.test")
        });
        assert!(looked.is_err(), "{name}: a lookup was made");
        if !*recorded {
            continue;
        }
        // Add Account is the window, with no menu, and so is a sign-in asked for by name.
        assert_eq!(names(&add_rows(&dom)), ["Other Account\u{2026}"], "{name}");
        pick_add(&dom, "Other Account\u{2026}");
        settle_quiet_seen(&mut dom).await;
        dom.in_scope(ScopeId::APP, || {
            super::direct::start(Issuer::Google, Mode::Add, Join::OnScreen);
        });
        settle_quiet_seen(&mut dom).await;
        assert_eq!(
            *windows.0.lock().unwrap(),
            [Ask::default(), Ask::default()],
            "{name}: Add Account and Google each open the window"
        );
    }
}
