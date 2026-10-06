//! The add-account window on Blitz, driven the way a person drives it, over fakes that count: a
//! whole password add and a whole browser add through quire's parts and porter's service, Escape
//! cancelling, a failure and Try Again, and the markup lint. The window is the real one
//! (`mail_app::ui::native::add_account_root`); the lookup, the browser sign-in, the browser and the
//! add are fakes, the store is a `TempDir`'s, and nothing reaches the network or a keyring.

use ds_harness::{Clock, Driver, Harness, HarnessConfig, Viewport};
use mail_app::ui::native::{
    AddAccountOpened, AddAccountSeams, AddAccountWiring, AddRequest, add_account_root,
};
use mail_core::discover::{Found, Source};
use mail_domain::presets;
use mail_store::SqliteStore;
use porter_core::{Credential, SecretText};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

#[path = "support/drive.rs"]
mod drive;
use drive::{Drive, Key};

const AUTHORIZE: &str = "https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz";

/// What the fakes were asked.
#[derive(Default)]
struct Log {
    lookups: Mutex<Vec<String>>,
    adds: Mutex<Vec<(String, Option<String>, bool)>>,
}

/// How each seam answers.
#[derive(Clone)]
struct Script {
    add: Result<String, String>,
    /// A browser sign-in finishes when this is notified.
    release: Arc<Notify>,
    /// A lookup waits while a test holds this, to look at the step that is working.
    hold: Arc<Mutex<()>>,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            add: Ok("added".to_owned()),
            release: Arc::new(Notify::new()),
            hold: Arc::default(),
        }
    }
}

/// Seams that reach nothing, answering as `script` says and counting in `log`.
fn seams(script: &Script, log: &Arc<Log>) -> AddAccountSeams {
    let (s, l) = (script.clone(), log.clone());
    let lookup = move |address: &str| {
        let _held = s.hold.lock().unwrap();
        l.lookups.lock().unwrap().push(address.to_owned());
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
    let add = move |request: AddRequest| {
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
    AddAccountSeams {
        lookup: Arc::new(lookup),
        jmap: Arc::new(|_| Err("no jmap".to_owned())),
        client: Arc::new(|_| true),
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
    open_in(script, prefill, ds::prelude::Theme::Light)
}

fn open_in(script: Script, prefill: Option<&str>, theme: ds::prelude::Theme) -> Window {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let log = Arc::new(Log::default());
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = opened.clone();
    let browse: Arc<mail_app::ui::native::AddAccountBrowse> = Arc::new(move |url| {
        seen.lock().unwrap().push(url.to_owned());
        Ok(())
    });
    let wiring = AddAccountWiring::new(seams(&script, &log), browse);
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Virtual)
        .with_context(store)
        .with_context(environment(theme))
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

fn step(w: &Window) -> Option<String> {
    let html = w.harness.html();
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

#[test]
fn the_window_opens_on_the_list_of_providers_with_round_marks_and_a_title_of_its_own() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    let html = w.harness.html();
    assert!(html.contains("Add Account"), "{html}");
    for name in [
        "Google",
        "Microsoft",
        "Fastmail",
        "iCloud",
        "Yahoo",
        "GMX",
        "Other",
    ] {
        assert!(html.contains(name), "{name} is not offered:\n{html}");
    }
    assert!(
        !html.contains("Email (IMAP)"),
        "the generic row is the list's Other…"
    );
    assert!(w.log.lookups.lock().unwrap().is_empty());
}

#[test]
fn a_whole_password_add() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    // The address field has the keyboard; the password follows with Tab.
    type_text(&mut w, "ada@example.test");
    w.harness.key(Key::Tab);
    type_text(&mut w, SECRET);
    assert!(
        !w.harness.html().contains(SECRET),
        "the password reached the markup"
    );
    w.harness.key(Key::Enter);
    on_step(&mut w, "review");
    assert!(
        w.log.adds.lock().unwrap().is_empty(),
        "nothing is added before the last button"
    );
    assert!(w.harness.html().contains("ada@example.test"));
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
}

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
    until(&mut w, "Copied", |h| h.html().contains("Copied"));

    click_text(&mut w, ".ds-acc-actions > :nth-child(3)");
    let opened = w.opened.clone();
    until(&mut w, "the page was not opened again", |_| {
        opened.lock().unwrap().len() == 2
    });

    w.script.release.notify_one();
    on_step(&mut w, "review");
    w.harness.key(Key::Enter);
    let log = w.log.clone();
    until(&mut w, "the add did not happen", |_| {
        !log.adds.lock().unwrap().is_empty()
    });
    assert_eq!(
        *w.log.adds.lock().unwrap(),
        [("ada@gmail.com".to_owned(), None, true)]
    );
}

#[test]
fn escape_cancels_and_adds_nothing() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    type_text(&mut w, "ada@example.test");
    w.harness.key(Key::Escape);
    // The service ended: no step is drawn, and nothing was looked up or added.
    until(&mut w, "the sheet did not end", |h| {
        h.html().contains("data-step=\"none\"")
    });
    assert!(w.log.lookups.lock().unwrap().is_empty());
    assert!(w.log.adds.lock().unwrap().is_empty());
}

#[test]
fn a_failed_step_offers_try_again_which_asks_the_form_again() {
    let script = Script {
        add: Err("cannot save the account".to_owned()),
        ..Script::default()
    };
    let mut w = open(script, Some("ada@example.test"));
    on_step(&mut w, "providers");
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    assert!(
        w.harness.html().contains("ada@example.test"),
        "the address being signed in again is already typed"
    );
    w.harness.key(Key::Tab);
    type_text(&mut w, SECRET);
    w.harness.key(Key::Enter);
    on_step(&mut w, "review");
    w.harness.key(Key::Enter);
    on_step(&mut w, "failed");
    assert_eq!(step(&w).as_deref(), Some("failed"));
    click_text(&mut w, ".ds-acc-actions > :nth-child(3)");
    on_step(&mut w, "sign-in");
    assert!(
        !w.harness.html().contains(SECRET),
        "the password did not survive the reply"
    );
}

#[test]
fn what_the_window_draws_passes_the_markup_lint() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    let providers = w.harness.html();
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    let form = w.harness.html();
    let css = format!(
        "{}\n{}",
        ds_shell::stylesheet(),
        include_str!("../src/ui/style/accounts.css")
    );
    let config = ds_lint::LintConfig::new(&ds_shell::kits());
    for (name, html) in [("providers", providers), ("sign-in", form)] {
        let offences = ds_lint::markup(&html, &css, &config);
        assert!(offences.is_empty(), "{name}: {offences:#?}");
    }
}

/// A picture of each step, light and dark, painted headlessly by Blitz: set `MAILO_SHOTS` to the
/// directory to write them to and run with `--ignored`.
#[test]
#[ignore = "picture generator: set MAILO_SHOTS to a directory and run with --ignored"]
fn render_each_step_to_files() {
    let dir = std::path::PathBuf::from(
        std::env::var("MAILO_SHOTS").expect("MAILO_SHOTS names the directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    for (name, theme) in [
        ("light", ds::prelude::Theme::Light),
        ("dark", ds::prelude::Theme::Dark),
    ] {
        let shoot = |w: &mut Window, step: &str| {
            w.harness.advance(Duration::from_millis(600));
            w.harness
                .render()
                .unwrap()
                .save(dir.join(format!("add-account-{step}-{name}.png")))
                .unwrap();
        };
        let script = Script::default();
        let hold = script.hold.clone();
        let mut w = open_in(script, None, theme);
        on_step(&mut w, "providers");
        shoot(&mut w, "providers");
        type_text(&mut w, "fast");
        w.harness.key(Key::Enter);
        on_step(&mut w, "sign-in");
        type_text(&mut w, "ada@example.test");
        shoot(&mut w, "sign-in");
        w.harness.key(Key::Tab);
        type_text(&mut w, SECRET);
        // The lookup is held, so the step that waits on it can be seen.
        let held = hold.lock().unwrap();
        w.harness.key(Key::Enter);
        on_step(&mut w, "working");
        shoot(&mut w, "working");
        drop(held);
        on_step(&mut w, "review");
        shoot(&mut w, "review");

        let script = Script {
            add: Err("cannot save the account".to_owned()),
            ..Script::default()
        };
        let mut w = open_in(script, None, theme);
        on_step(&mut w, "providers");
        type_text(&mut w, "fast");
        w.harness.key(Key::Enter);
        on_step(&mut w, "sign-in");
        type_text(&mut w, "ada@example.test");
        w.harness.key(Key::Tab);
        type_text(&mut w, SECRET);
        w.harness.key(Key::Enter);
        on_step(&mut w, "review");
        w.harness.key(Key::Enter);
        on_step(&mut w, "failed");
        shoot(&mut w, "failed");

        let mut w = open_in(Script::default(), None, theme);
        on_step(&mut w, "providers");
        type_text(&mut w, "google");
        w.harness.key(Key::Enter);
        on_step(&mut w, "sign-in");
        type_text(&mut w, "ada@gmail.com");
        w.harness.key(Key::Enter);
        on_step(&mut w, "browser");
        shoot(&mut w, "browser");
    }
}
