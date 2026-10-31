//! The add-account window on Blitz, driven the way a person drives it, over fakes that count: a
//! whole password add, a whole browser add and a whole add of servers typed by hand through
//! quire's parts and porter's service; Escape cancelling, a failure and Try Again; the size each
//! step asks of the window; and the markup lint. The window is the real one
//! (`mail_ui::ui::native::add_account_root`); the lookup, the browser sign-in, the browser and the
//! add are fakes, the store is a `TempDir`'s, and nothing reaches the network or a keyring.

use ds_blitz::Extent;
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};
use mail_core::SqliteStore;
use mail_core::discover::{Failed, Found, Gap, Source};
use mail_domain::presets;
use mail_ui::ui::native::{AddAccountOpened, AddAccountWiring, add_account_root};
use mail_ui_model::add_account::provider::{Request, Seams};
use porter_core::{Credential, SecretText};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

use crate::drive;
use drive::{Drive, Key};

const AUTHORIZE: &str = "https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz";

/// What the fakes were asked.
#[derive(Default)]
struct Log {
    lookups: Mutex<Vec<String>>,
    adds: Mutex<Vec<(String, Option<String>, bool)>>,
    /// What the add was told to set up, as a word per kind of server.
    setups: Mutex<Vec<String>>,
}

/// How each seam answers.
#[derive(Clone)]
struct Script {
    add: Result<String, mail_ui_model::add_account::provider::AddError>,
    /// A browser sign-in finishes when this is notified.
    release: Arc<Notify>,
    /// A lookup waits while a test holds this, to look at the step that is working.
    hold: Arc<Mutex<()>>,
    /// Whether a lookup finds servers: when it does not, the person types them.
    found: bool,
    /// Whether the build has a client for a browser provider: without one, the sign-in fails.
    client: bool,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            add: Ok("added".to_owned()),
            release: Arc::new(Notify::new()),
            hold: Arc::default(),
            found: true,
            client: true,
        }
    }
}

/// Seams that reach nothing, answering as `script` says and counting in `log`.
fn seams(script: &Script, log: &Arc<Log>) -> Seams {
    let (s, l) = (script.clone(), log.clone());
    let lookup = move |address: &str| {
        let _held = s.hold.lock().unwrap();
        l.lookups.lock().unwrap().push(address.to_owned());
        if !s.found {
            return Err(Failed::NoServers {
                address: address.to_owned(),
                gap: Gap::Nothing,
                tried: String::new(),
            });
        }
        let manual = presets::Manual {
            imap_host: "imap.example.test".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.example.test".to_owned(),
            smtp_port: 465,
            login: None,
        };
        Ok(Found {
            source: Source::Autoconfig,
            preset: presets::manual(address, &manual, chrono::Utc::now()),
        })
    };
    let (s, l) = (script.clone(), log.clone());
    let add = move |request: Request| {
        l.setups
            .lock()
            .unwrap()
            .push(format!("{:?}", request.setup));
        l.adds.lock().unwrap().push((
            request.address,
            request
                .password
                .map(|password| password.expose().to_owned()),
            request.signed.is_some(),
        ));
        s.add.clone()
    };
    let s = script.clone();
    Seams {
        lookup: Arc::new(lookup),
        jmap: Arc::new(|_| Err("no jmap".to_owned())),
        client: Arc::new({
            let client = script.client;
            move |_| client
        }),
        authorize: Arc::new(move |_, _, urls| {
            let s = s.clone();
            Box::pin(async move {
                urls(AUTHORIZE);
                s.release.notified().await;
                Ok(Credential::Password(SecretText::new("refresh-token")))
            })
        }),
        add: Arc::new(add),
    }
}

const VIEW: Viewport = Viewport {
    width: 520,
    height: 700,
    scale_percent: 100,
};

/// The desktop's look, fixed: `theme`, and the system's own preferences as a test has them.
fn environment(theme: ds::prelude::Theme) -> ds_settings::Environment {
    let mut environment = ds_settings::Environment::default();
    environment.settings.appearance.theme = theme;
    environment
}

const SECRET: &str = "s3cretpass4417";

struct Window {
    harness: Harness,
    log: Arc<Log>,
    opened: Arc<Mutex<Vec<String>>>,
    script: Script,
    _dir: tempfile::TempDir,
}

fn open(script: Script, prefill: Option<&str>) -> Window {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let log = Arc::new(Log::default());
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = opened.clone();
    let browse: Arc<mail_ui::ui::native::AddAccountBrowse> = Arc::new(move |url| {
        seen.lock().unwrap().push(url.to_owned());
        Ok(())
    });
    let wiring = AddAccountWiring::new(seams(&script, &log), browse);
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Virtual)
        .with_context(mail_client::Client::inline(
            store,
            Arc::new(mail_core::mail::SystemClock),
        ))
        .with_context(environment(ds::prelude::Theme::Light))
        .with_context(AddAccountOpened::new(wiring, prefill.map(str::to_owned)));
    let mut harness = Harness::new(add_account_root, config);
    harness.advance(Duration::from_millis(300));
    Window {
        harness,
        log,
        opened,
        script,
        _dir: dir,
    }
}

/// Let the service and the blocking work land, until `done` holds.
fn until(w: &mut Window, what: &str, done: impl Fn(&Harness) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if done(&w.harness) {
            return;
        }
        w.harness.advance(Duration::from_millis(20));
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("{what}:\n{}", w.harness.html());
}

fn step_of(h: &Harness) -> Option<String> {
    let html = h.html();
    let at = html.find("data-step=\"")? + "data-step=\"".len();
    Some(html[at..].split('"').next()?.to_owned())
}

fn on_step(w: &mut Window, slug: &str) {
    until(w, &format!("never reached the {slug} step"), |h| {
        h.html().contains(&format!(
            "class=\"add-account-window\" data-step=\"{slug}\""
        )) || h.html().contains(&format!("data-step=\"{slug}\""))
    });
}

fn click_text(w: &mut Window, selector: &str) {
    let at = w
        .harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", w.harness.html()));
    w.harness.click(at);
    w.harness.advance(Duration::from_millis(100));
}

fn type_text(w: &mut Window, text: &str) {
    for c in text.chars() {
        w.harness
            .key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
    w.harness.advance(Duration::from_millis(100));
}

/// The markup lint's offences in `html`, against the window's own styles.
fn lint(html: &str) -> Vec<ds_lint::Offence> {
    let css = format!(
        "{}\n{}",
        ds_shell::stylesheet(),
        include_str!("../../../mail-ui/src/ui/style/accounts.css")
    );
    ds_lint::markup(html, &css, &ds_lint::LintConfig::new(&ds_shell::kits()))
}

/// What the window asked for, each size once in a row.
fn asked(w: &Window) -> Vec<Extent> {
    let mut sizes: Vec<Extent> = Vec::new();
    for size in w.harness.window_requests().iter() {
        if sizes.last() != Some(size) {
            sizes.push(*size);
        }
    }
    sizes
}

/// The protocol's pop-up, first in the form of typed servers, which no other step has.
const PROTOCOL: &str = ".ds-form > .ds-form-section:first-child .ds-popup .ds-button";

/// Pick "Other…", type an address and a password: the lookup finds no server (`found: false`)
/// and the form of servers typed by hand comes.
fn to_server_form(w: &mut Window) {
    on_step(w, "providers");
    // Nothing matches, so the cursor is on the list's own "Other…".
    type_text(w, "zz");
    w.harness.key(Key::Enter);
    on_step(w, "sign-in");
    type_text(w, "ada@example.test");
    w.harness.key(Key::Tab);
    type_text(w, SECRET);
    w.harness.key(Key::Enter);
    until(w, "the form of typed servers never came", |h| {
        h.count(PROTOCOL) == 1
    });
}

/// Choose the `nth` (from 1) of the protocol's options in its pop-up.
fn pick_protocol(w: &mut Window, nth: usize) {
    click_text(w, PROTOCOL);
    until(w, "the protocol's menu never opened", |h| {
        h.count(".ds-menu") == 1
    });
    click_text(w, &format!(".ds-menu .ds-menu-item:nth-child({nth})"));
    w.harness.advance(Duration::from_millis(200));
}

fn no_servers() -> Script {
    Script {
        found: false,
        ..Script::default()
    }
}

/// The sheet after a password sign-in whose add fails with `fault`, the server's words `said`.
fn add_fails(fault: mail_ui_model::add_account::provider::AddFault, said: &str) -> Window {
    use mail_ui_model::add_account::provider::AddError;
    let script = Script {
        add: Err(AddError {
            fault,
            detail: said.to_owned(),
        }),
        ..Script::default()
    };
    let mut w = open(script, Some("ada@example.test"));
    on_step(&mut w, "providers");
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    w.harness.key(Key::Tab);
    type_text(&mut w, SECRET);
    w.harness.key(Key::Enter);
    on_step(&mut w, "failed");
    w
}

/// The window opens on the list of providers, looking nothing up, and asks for its size; the
/// sign-in form is shorter. The address field has the keyboard and the password follows with
/// Tab, never reaching the markup; with nothing to choose, Return adds the account, no empty
/// review between. Both steps pass the markup lint.
#[test]
fn a_whole_password_add_looks_nothing_up_from_the_list_and_passes_the_lint() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    let providers = w.harness.html();
    assert!(
        w.log.lookups.lock().unwrap().is_empty(),
        "the list looked something up"
    );
    let list = asked(&w);
    assert_eq!(list.len(), 1, "{list:?}");

    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    let form = w.harness.html();
    let short = *asked(&w).last().unwrap();
    assert!(short.height < list[0].height, "{short:?} against {list:?}");
    for (name, html) in [("providers", providers), ("sign-in", form)] {
        let offences = lint(&html);
        assert!(offences.is_empty(), "the lint on {name}: {offences:#?}");
    }

    type_text(&mut w, "ada@example.test");
    w.harness.key(Key::Tab);
    type_text(&mut w, SECRET);
    assert!(
        !w.harness.html().contains(SECRET),
        "the password reached the markup"
    );
    w.harness.key(Key::Enter);
    let log = w.log.clone();
    until(&mut w, "the add did not happen", |h| {
        assert_ne!(
            step_of(h).as_deref(),
            Some("review"),
            "an empty review was drawn"
        );
        !log.adds.lock().unwrap().is_empty()
    });
    assert_eq!(
        *w.log.adds.lock().unwrap(),
        [(
            "ada@example.test".to_owned(),
            Some(SECRET.to_owned()),
            false
        )]
    );
}

/// A browser provider opens the page in the browser and shows it to copy; the first action
/// copies it and the last opens it again; the add comes once the sign-in finishes. Built with
/// no client for the provider, the failure offers no Try Again, and Back goes back to the list.
#[test]
fn a_whole_browser_add_opens_the_page_copies_the_link_and_adds_when_signed_in() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    type_text(&mut w, "google");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    type_text(&mut w, "ada@gmail.com");
    w.harness.key(Key::Enter);
    on_step(&mut w, "browser");
    assert_eq!(
        *w.opened.lock().unwrap(),
        [AUTHORIZE],
        "the browser was opened with the page"
    );
    assert!(
        w.harness.html().contains(AUTHORIZE),
        "the page is shown to copy"
    );
    click_text(&mut w, ".ds-acc-actions > :nth-child(1)");
    assert_eq!(w.harness.clipboard_text().as_deref(), Some(AUTHORIZE));
    click_text(&mut w, ".ds-acc-actions > :nth-child(3)");
    let opened = w.opened.clone();
    until(&mut w, "the page was not opened again", |_| {
        opened.lock().unwrap().len() == 2
    });
    w.script.release.notify_one();
    let log = w.log.clone();
    until(&mut w, "the add did not happen", |_| {
        !log.adds.lock().unwrap().is_empty()
    });
    assert_eq!(
        *w.log.adds.lock().unwrap(),
        [("ada@gmail.com".to_owned(), None, true)]
    );

    let script = Script {
        client: false,
        ..Script::default()
    };
    let mut w = open(script, None);
    on_step(&mut w, "providers");
    type_text(&mut w, "google");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    type_text(&mut w, "ada@gmail.com");
    w.harness.key(Key::Enter);
    on_step(&mut w, "failed");
    assert_eq!(
        w.harness.count(".ds-acc-actions > :nth-child(3)"),
        0,
        "a retry that cannot help is offered"
    );
    click_text(&mut w, ".ds-acc-actions > :nth-child(1)");
    on_step(&mut w, "providers");
}

/// Escape ends the sheet having looked up and added nothing. An add that fails offers Try
/// Again, which asks the form again with the address kept and the password not; a refused
/// login says the server's words under a sentence of its own, in a window grown to hold them.
#[test]
fn escape_cancels_and_a_failed_add_says_why_and_tries_again() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    type_text(&mut w, "ada@example.test");
    w.harness.key(Key::Escape);
    on_step(&mut w, "none");
    assert!(w.log.lookups.lock().unwrap().is_empty());
    assert!(w.log.adds.lock().unwrap().is_empty());

    use mail_ui_model::add_account::provider::AddFault;
    let said = "mail.example.test did not accept the sign-in as ada@example.test: \
                -ERR [AUTH] Authentication failed.";
    let mut w = add_fails(AddFault::Unsaved, said);
    let unsaved = w.harness.text_of(".ds-acc-caption");
    click_text(&mut w, ".ds-acc-actions > :nth-child(3)");
    on_step(&mut w, "sign-in");
    let html = w.harness.html();
    assert!(
        html.contains("ada@example.test"),
        "the address was not kept"
    );
    assert!(!html.contains(SECRET), "the password survived the failure");

    let mut w = add_fails(AddFault::Refused, said);
    let refused = w.harness.text_of(".ds-acc-caption");
    assert!(
        refused.as_deref().is_some_and(|s| !s.is_empty()),
        "no sentence for a refusal"
    );
    assert_ne!(refused, unsaved, "a refused login is not a failure to save");
    assert_eq!(
        w.harness.text_of(".add-account-detail").as_deref(),
        Some(said),
        "the server's words under the sentence"
    );
    assert!(
        !w.harness.html().contains(SECRET),
        "the password reached the markup"
    );
    w.harness.advance(Duration::from_millis(300));
    let failed = asked(&w).last().copied().unwrap();
    let detail = w.harness.rect(".add-account-detail").unwrap();
    let bottom = detail.origin.y.0 + detail.size.height.0;
    assert!(
        bottom <= failed.height as f32,
        "{failed:?} has no room for what ends at {bottom}"
    );
}

/// When the lookup finds nothing, the person types the servers: a larger form, IMAP first with
/// the guessed hosts. JMAP drops the outgoing part, and the window shrinks for it; both forms
/// pass the lint. POP3 changes the form at once, looking nothing up, and the add sets up what
/// was typed. A port that is no number is marked on its field and nothing is sent.
#[test]
fn a_whole_add_of_typed_servers_follows_the_protocol_and_refuses_a_wrong_port() {
    let mut w = open(no_servers(), None);
    to_server_form(&mut w);
    let imap = w.harness.html();
    assert!(
        imap.contains("imap.example.test") && imap.contains("smtp.example.test"),
        "{imap}"
    );
    assert!(!imap.contains(SECRET), "the password reached the markup");
    assert_eq!(w.log.lookups.lock().unwrap().len(), 1);
    let server = *asked(&w).last().unwrap();

    pick_protocol(&mut w, 3);
    let jmap = w.harness.html();
    assert!(
        !jmap.contains("smtp.example.test"),
        "JMAP has an outgoing part:\n{jmap}"
    );
    assert!(
        asked(&w).last().unwrap().height < server.height,
        "the window kept its height"
    );
    for (name, html) in [("imap", imap), ("jmap", jmap)] {
        let offences = lint(&html);
        assert!(offences.is_empty(), "the lint on {name}: {offences:#?}");
    }
    pick_protocol(&mut w, 1);
    assert_eq!(*asked(&w).last().unwrap(), server, "and back");

    pick_protocol(&mut w, 2);
    let html = w.harness.html();
    assert!(
        html.contains("pop.example.test") && html.contains("995"),
        "{html}"
    );
    assert_eq!(
        w.log.lookups.lock().unwrap().len(),
        1,
        "a pick looks nothing up"
    );
    // After a pick the keyboard is on nothing: Return belongs to the field clicked into.
    click_text(&mut w, ".add-account-step input");
    w.harness.key(Key::Enter);
    let log = w.log.clone();
    until(&mut w, "the add did not happen", |_| {
        !log.adds.lock().unwrap().is_empty()
    });
    assert_eq!(
        *w.log.adds.lock().unwrap(),
        [(
            "ada@example.test".to_owned(),
            Some(SECRET.to_owned()),
            false
        )]
    );
    let setup = w.log.setups.lock().unwrap()[0].clone();
    assert!(
        setup.contains("Pop3") && setup.contains("pop.example.test") && setup.contains("995"),
        "{setup}"
    );

    // The port has a usual answer, so it waits under Advanced.
    let mut w = open(no_servers(), None);
    to_server_form(&mut w);
    click_text(&mut w, ".ds-disclosure-trigger");
    w.harness.advance(Duration::from_millis(300));
    let port = ".ds-disclosure input";
    click_text(&mut w, port);
    type_text(&mut w, "x");
    w.harness.key(Key::Enter);
    w.harness.advance(Duration::from_millis(300));
    assert_eq!(
        w.harness.attr(port, "aria-invalid").as_deref(),
        Some("true"),
        "the port is not marked:\n{}",
        w.harness.html()
    );
    assert_eq!(
        step_of(&w.harness).as_deref(),
        Some("sign-in"),
        "something was sent"
    );
    assert!(w.log.adds.lock().unwrap().is_empty());
}

/// An input method's preedit is drawn in the field before it is the field's text: the
/// placeholder goes as composing starts, stays gone while it runs, and the commit is the text.
#[test]
fn pinyin_being_composed_hides_the_placeholder_and_its_commit_is_the_text() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    const PLACEHOLDER: &str = ".ds-input-placeholder";
    let before = w.harness.count(PLACEHOLDER);
    assert!(before > 0, "the sign-in fields have no placeholder");
    w.harness.ime_start();
    w.harness.ime_update("ni", 2);
    w.harness.advance(Duration::from_millis(100));
    assert_eq!(
        w.harness.count(PLACEHOLDER),
        before - 1,
        "the placeholder stayed over the preedit"
    );
    w.harness.ime_commit("你");
    w.harness.ime_end();
    w.harness.advance(Duration::from_millis(100));
    assert_eq!(
        w.harness.count(PLACEHOLDER),
        before - 1,
        "the committed text has the placeholder over it"
    );
    assert!(
        w.harness.html().contains("value=\"你\""),
        "the commit is not the text"
    );
}
