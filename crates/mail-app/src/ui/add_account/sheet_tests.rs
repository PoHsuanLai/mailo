//! The sheet as drawn and pressed, over fakes that count: Look up shows what was found and adds
//! nothing, Use these settings adds once, Cancel adds nothing, and the password is gone from
//! everything the window holds afterwards.

use std::sync::{Arc, Mutex};

use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_proto::discover::Found;
use mail_store::SqliteStore;

use super::AddAccountSheet;
use super::flow::Seams;
use super::flow_tests::{Fake, found, seams};
use crate::space::{Scope, Space, Spaces};
use crate::ui::fixtures::{Scripts, Seen, click, dispatching, rebuild_into, type_into};
use crate::view::Shell;

const PASSWORD: &str = "s3cret-pass-4417";

/// The window's state after each render, as `{:?}` would print it: the shell, the Spaces and
/// the revision. Compared by identity, so the props never change.
#[derive(Clone, Default)]
struct Snapshot(Arc<Mutex<String>>);

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Snapshot {
    fn get(&self) -> String {
        self.0.lock().unwrap().clone()
    }
}

/// The sheet open, alone, in a Space scoped to `scope` (empty: every account).
#[component]
fn Sheet(snapshot: Snapshot, scope: Vec<mail_domain::AccountId>) -> Element {
    let shell = use_signal(|| Shell {
        adding: Some(String::new()),
        ..Shell::default()
    });
    let revision = use_signal(|| 0u64);
    let spaces = use_signal(|| Spaces {
        spaces: vec![Space {
            scope: if scope.is_empty() {
                Scope::All
            } else {
                Scope::Accounts(scope.clone())
            },
            ..Space::default()
        }],
        ..Spaces::default()
    });
    *snapshot.0.lock().unwrap() = format!(
        "{:?}\n{:?}\nrevision {}",
        shell.read(),
        spaces.read(),
        revision()
    );
    rsx! {
        if shell.read().adding.is_some() {
            AddAccountSheet { shell, revision, spaces }
        }
    }
}

struct Open {
    dom: VirtualDom,
    seen: Seen,
    snapshot: Snapshot,
    /// Every script the sheet ran: the clipboard's, for one.
    scripts: Scripts,
}

fn open(store: &Arc<SqliteStore>, seams: Seams, scope: Vec<mail_domain::AccountId>) -> Open {
    let snapshot = Snapshot::default();
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new_with_props(
        Sheet,
        SheetProps {
            snapshot: snapshot.clone(),
            scope,
        },
    )
    .with_root_context(store.clone())
    .with_root_context(seams)
    .with_root_context(scripts.document());
    let seen = rebuild_into(&mut dom);
    Open {
        dom,
        seen,
        snapshot,
        scripts,
    }
}

/// Let the blocking work land and draw it, keeping every attribute set on the way.
async fn settle(dom: &mut VirtualDom) -> Seen {
    let mut seen = Seen::default();
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
    seen
}

/// Type `address` and press Look up; returns the render after the lookup landed.
async fn look_up(open: &mut Open, address: &str) -> Seen {
    let field = open.seen.one("placeholder", "you@example.com");
    type_into(&mut open.dom, field, address);
    // What the click itself drew is kept: a quick answer can land in that same render, and its
    // button is then drawn there and nowhere later. After that, work is waited for rather than a
    // quiet spell while the sheet says it is looking, since on a loaded machine a spell can pass
    // with the answer still on its way.
    let mut seen = click(&mut open.dom, open.seen.one("aria-label", "Look up"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while page(open).contains("Looking up the servers for") && std::time::Instant::now() < deadline
    {
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(1), open.dom.wait_for_work()).await;
        open.dom.render_immediate(&mut seen);
    }
    seen.merge(settle(&mut open.dom).await)
}

fn page(open: &Open) -> String {
    dioxus_ssr::render(&open.dom)
}

fn store() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Arc::new(SqliteStore::in_memory(dir.path()).unwrap()), dir)
}

fn ok(address: &str) -> Result<Found, String> {
    Ok(found(address))
}

#[tokio::test]
async fn look_up_shows_what_was_found_and_adds_nothing_until_it_is_used() {
    dispatching();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let mut open = open(
        &store,
        seams(&fake, ok("ada@example.test"), false),
        Vec::new(),
    );
    assert!(page(&open).contains("only the domain"), "{}", page(&open));

    let seen = look_up(&mut open, "ada@example.test").await;
    let shown = page(&open);
    assert!(shown.contains("IMAP imap.example.test:993"), "{shown}");
    assert!(shown.contains("SMTP smtp.example.test:465"), "{shown}");
    assert!(shown.contains("from the Thunderbird ISPDB"), "{shown}");
    assert!(shown.contains("type=\"password\""), "{shown}");
    assert_eq!(
        (fake.looked(), fake.added()),
        (1, 0),
        "looking added something"
    );
    assert!(crate::ui::data::account_rows(&store).is_empty());

    let secret = seen.one("placeholder", "Password for ada@example.test");
    type_into(&mut open.dom, secret, PASSWORD);
    assert_eq!(fake.added(), 0, "typing the password added something");
    click(&mut open.dom, seen.one("aria-label", "Use these settings"));
    settle(&mut open.dom).await;

    assert_eq!(fake.added(), 1);
    let shown = page(&open);
    assert!(shown.contains("Added ada@example.test."), "{shown}");
    let rows = crate::ui::data::account_rows(&store);
    assert_eq!(rows.len(), 1);
    assert!(
        open.snapshot.get().contains("revision 1"),
        "the window was not told"
    );

    // The password reached the add, and the keyring fake, and nothing else the window holds.
    assert_eq!(*fake.handed.lock().unwrap(), [PASSWORD]);
    assert_eq!(fake.kept(rows[0].id).as_deref(), Some(PASSWORD));
    assert!(!shown.contains(PASSWORD), "the page holds the password");
    assert!(
        !open.snapshot.get().contains(PASSWORD),
        "the shell or the Spaces hold the password"
    );
}

#[tokio::test]
async fn cancel_adds_nothing() {
    dispatching();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let mut open = open(
        &store,
        seams(&fake, ok("ada@example.test"), false),
        Vec::new(),
    );
    let seen = look_up(&mut open, "ada@example.test").await;
    type_into(
        &mut open.dom,
        seen.one("placeholder", "Password for ada@example.test"),
        PASSWORD,
    );
    // Drawn with the sheet and never changed since, so found in the first render.
    click(&mut open.dom, open.seen.one("aria-label", "Cancel"));
    settle(&mut open.dom).await;
    assert!(open.snapshot.get().contains("adding: None"), "still open");
    assert_eq!(fake.added(), 0);
    assert!(crate::ui::data::account_rows(&store).is_empty());
    assert!(!open.snapshot.get().contains(PASSWORD));
}

#[tokio::test]
async fn a_lookup_error_is_shown_in_the_sheet() {
    dispatching();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let why = "could not find servers for ada@nowhere.test: nothing answered";
    let mut open = open(&store, seams(&fake, Err(why.to_owned()), false), Vec::new());
    look_up(&mut open, "ada@nowhere.test").await;
    let shown = page(&open);
    assert!(shown.contains(why), "{shown}");
    assert!(shown.contains("role=\"alert\""), "{shown}");
    assert_eq!(fake.added(), 0);
}

#[tokio::test]
async fn an_oauth_account_offers_the_browser_sign_in_and_no_password_field() {
    dispatching();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let mut open = open(
        &store,
        seams(&fake, Err("unused".to_owned()), true),
        Vec::new(),
    );
    let seen = look_up(&mut open, "ada@gmail.com").await;
    let shown = page(&open);
    assert!(
        seen.get("aria-label", "Sign in with Google").is_some(),
        "{shown}"
    );
    assert!(!shown.contains("type=\"password\""), "{shown}");
    assert!(shown.contains("from the built-in table"), "{shown}");
    assert_eq!((fake.looked(), fake.added()), (0, 0));
}

#[tokio::test]
async fn a_new_account_joins_a_scoped_space() {
    dispatching();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let other = mail_domain::AccountId::generate();
    let mut open = open(
        &store,
        seams(&fake, ok("ada@example.test"), false),
        vec![other],
    );
    let seen = look_up(&mut open, "ada@example.test").await;
    type_into(
        &mut open.dom,
        seen.one("placeholder", "Password for ada@example.test"),
        PASSWORD,
    );
    click(&mut open.dom, seen.one("aria-label", "Use these settings"));
    settle(&mut open.dom).await;
    let added = crate::ui::data::account_rows(&store)[0].id;
    let state = open.snapshot.get();
    assert!(
        state.contains(&format!("Accounts([{other:?}, {added:?}])")),
        "{state}"
    );
    assert!(
        state.contains(&format!("scope: [{other:?}, {added:?}]")),
        "the shell's scope did not follow: {state}"
    );
}

/// The address a fake sign-in waits on. No `&`, so the page holds it as written.
const SIGN_IN: &str = "https://accounts.example.test/o/oauth2/auth?client_id=abc";

/// Seams over `fake` whose add hands over [`SIGN_IN`] and then waits in the browser until the
/// returned sender says the sign-in was given up. The browser is `fake`'s, which only writes the
/// address down.
fn signing_in(fake: &Arc<Fake>) -> (Seams, std::sync::mpsc::Sender<()>) {
    let (give_up, given_up) = std::sync::mpsc::channel::<()>();
    let given_up = Mutex::new(given_up);
    let mut seams = seams(fake, Err("unused".to_owned()), true);
    seams.add = Arc::new(move |_, _, on_url| {
        on_url(SIGN_IN);
        let _ = given_up.lock().unwrap().recv();
        Err("the sign-in was abandoned".to_owned())
    });
    (seams, give_up)
}

/// The sheet over `seams` with Sign in with Google pressed, drawn until the sign-in's address
/// arrived from the blocking add or a few quiet spells passed; returns everything that drew.
async fn press_sign_in(store: &Arc<SqliteStore>, seams: Seams) -> (Open, Seen) {
    let mut open = open(store, seams, Vec::new());
    let seen = look_up(&mut open, "ada@gmail.com").await;
    let mut drawn = click(&mut open.dom, seen.one("aria-label", "Sign in with Google"));
    for _ in 0..20 {
        drawn = drawn.merge(settle(&mut open.dom).await);
        if page(&open).contains(SIGN_IN) {
            break;
        }
    }
    (open, drawn)
}

#[tokio::test]
async fn a_browser_sign_in_shows_its_address_to_copy_and_opens_it_through_the_seam() {
    dispatching();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let (seams, give_up) = signing_in(&fake);
    let (mut open, seen) = press_sign_in(&store, seams).await;

    let shown = page(&open);
    assert!(
        shown.contains("Finish signing in in your browser"),
        "{shown}"
    );
    assert!(shown.contains(&format!(">{SIGN_IN}<")), "{shown}");
    assert!(!shown.contains("printed in the terminal"), "{shown}");
    assert_eq!(*fake.browsed.lock().unwrap(), [SIGN_IN], "the browser seam");

    let before = open.scripts.all().len();
    click(&mut open.dom, seen.one("aria-label", "Copy"));
    let ran = open.scripts.all();
    assert_eq!(ran.len(), before + 1, "{ran:?}");
    assert!(
        ran[before].contains("navigator.clipboard.writeText")
            && ran[before].contains(&serde_json::to_string(SIGN_IN).unwrap()),
        "{ran:?}"
    );

    give_up.send(()).unwrap();
    settle(&mut open.dom).await;
    let shown = page(&open);
    assert!(
        shown.contains("Not added: the sign-in was abandoned"),
        "{shown}"
    );
    assert!(
        !shown.contains(SIGN_IN),
        "the address outlived the sign-in: {shown}"
    );
    assert_eq!(fake.browsed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_browser_that_will_not_open_leaves_the_address_to_open_by_hand() {
    dispatching();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let (mut seams, give_up) = signing_in(&fake);
    // The window's own seam, which under test refuses.
    seams.browse = Seams::real().browse;
    let (mut open, _) = press_sign_in(&store, seams).await;
    let shown = page(&open);
    assert!(
        shown.contains("mailo could not open a browser (no browser is opened in tests)"),
        "{shown}"
    );
    assert!(shown.contains(&format!(">{SIGN_IN}<")), "{shown}");
    give_up.send(()).unwrap();
    settle(&mut open.dom).await;
}

/// The sheet in each state worth looking at, as `(name, markup)`.
async fn every_state() -> Vec<(&'static str, String)> {
    let mut all = Vec::new();
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let blank = open(
        &store,
        seams(&fake, ok("ada@example.test"), false),
        Vec::new(),
    );
    all.push(("empty", page(&blank)));

    let mut found = open(
        &store,
        seams(&fake, ok("ada@example.test"), false),
        Vec::new(),
    );
    let seen = look_up(&mut found, "ada@example.test").await;
    all.push(("found", page(&found)));
    type_into(
        &mut found.dom,
        seen.one("placeholder", "Password for ada@example.test"),
        PASSWORD,
    );
    click(&mut found.dom, seen.one("aria-label", "Use these settings"));
    settle(&mut found.dom).await;
    all.push(("added", page(&found)));

    let mut oauth = open(&store, seams(&fake, ok("unused@x.test"), true), Vec::new());
    look_up(&mut oauth, "ada@gmail.com").await;
    all.push(("oauth", page(&oauth)));

    let (signs_in, give_up) = signing_in(&fake);
    let (mut waiting, _) = press_sign_in(&store, signs_in).await;
    all.push(("oauth-waiting", page(&waiting)));
    give_up.send(()).unwrap();
    settle(&mut waiting.dom).await;

    let mut missing = open(&store, seams(&fake, ok("unused@x.test"), false), Vec::new());
    look_up(&mut missing, "ada@gmail.com").await;
    all.push(("oauth-missing", page(&missing)));

    let why = "could not find servers for ada@nowhere.test: no autoconfig, no SRV record and \
               no MX this client knows\n\nName the servers yourself:\n\n  mailo account add \
               ada@nowhere.test --imap HOST[:993] --smtp HOST[:465] [--login NAME]";
    let mut error = open(&store, seams(&fake, Err(why.to_owned()), false), Vec::new());
    look_up(&mut error, "ada@nowhere.test").await;
    all.push(("error", page(&error)));
    all
}

#[tokio::test]
async fn every_class_the_add_account_sheet_draws_is_styled() {
    dispatching();
    let markup: String = every_state()
        .await
        .into_iter()
        .map(|(_, page)| page)
        .collect();
    for class in [
        "acct-found",
        "acct-k",
        "acct-source",
        "acct-done",
        "secret",
        "acct-said",
        "acct-url",
        "acct-link",
    ] {
        assert!(markup.contains(class), "{class} was not drawn");
    }
    let missing = crate::ui::style::tests::unstyled_classes(&markup, crate::ui::style::STYLE);
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
    assert!(!markup.contains(PASSWORD), "a state drew the password");
}

/// `extra` as the first child of `.app`, where the window mounts its overlays.
fn inject(page: &str, extra: &str) -> String {
    let at = page.find("class=\"app").unwrap_or(0);
    let close = page[at..].find('>').map_or(page.len(), |rel| at + rel + 1);
    format!("{}{extra}{}", &page[..close], &page[close..])
}

#[tokio::test]
#[ignore = "writes target/add-account-*.html and their -dark twins to look at"]
async fn render_the_add_account_sheet_to_files() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let mut frame = VirtualDom::new(crate::ui::app::App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    frame.rebuild_in_place();
    let backdrop = dioxus_ssr::render(&frame);
    for (name, page) in every_state().await {
        crate::ui::fixtures::dump(&format!("add-account-{name}"), &inject(&backdrop, &page));
    }
}
