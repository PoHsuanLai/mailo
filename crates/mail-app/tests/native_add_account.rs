//! The add-account window on Blitz, driven the way a person drives it, over fakes that count: a
//! whole password add and a whole browser add through quire's parts and porter's service, Escape
//! cancelling, a failure and Try Again, and the markup lint. The window is the real one
//! (`mail_app::ui::native::add_account_root`); the lookup, the browser sign-in, the browser and the
//! add are fakes, the store is a `TempDir`'s, and nothing reaches the network or a keyring.

use ds_blitz::Extent;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};
use mail_app::ui::native::{
    AddAccountOpened, AddAccountSeams, AddAccountWiring, AddRequest, add_account_root,
};
use mail_core::discover::{Failed, Found, Gap, Source};
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
    /// What the add was told to set up, as a word per kind of server.
    setups: Mutex<Vec<String>>,
}

/// How each seam answers.
#[derive(Clone)]
struct Script {
    add: Result<String, String>,
    /// A browser sign-in finishes when this is notified.
    release: Arc<Notify>,
    /// A lookup waits while a test holds this, to look at the step that is working.
    hold: Arc<Mutex<()>>,
    /// Whether a lookup finds servers: when it does not, the person types them.
    found: bool,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            add: Ok("added".to_owned()),
            release: Arc::new(Notify::new()),
            hold: Arc::default(),
            found: true,
        }
    }
}

/// Seams that reach nothing, answering as `script` says and counting in `log`.
fn seams(script: &Script, log: &Arc<Log>) -> AddAccountSeams {
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
    let add = move |request: AddRequest| {
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
    /// Every size the window asked its host for, in order.
    fits: Arc<Mutex<Vec<Extent>>>,
    log: Arc<Log>,
    opened: Arc<Mutex<Vec<String>>>,
    script: Script,
    _dir: tempfile::TempDir,
}

fn open(script: Script, prefill: Option<&str>) -> Window {
    open_in(script, prefill, ds::prelude::Theme::Light)
}

fn open_in(script: Script, prefill: Option<&str>, theme: ds::prelude::Theme) -> Window {
    open_sized(script, prefill, theme, VIEW)
}

/// The window in a viewport of `view`'s size (the harness's is fixed for its life).
fn open_sized(
    script: Script,
    prefill: Option<&str>,
    theme: ds::prelude::Theme,
    view: Viewport,
) -> Window {
    open_with(script, prefill, theme, view, None)
}

/// [`open_sized`], with the provider icons a window would have read from its cache.
fn open_with(
    script: Script,
    prefill: Option<&str>,
    theme: ds::prelude::Theme,
    view: Viewport,
    icons: Option<mail_core::provider::icon::Loaded>,
) -> Window {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let log = Arc::new(Log::default());
    let opened = Arc::new(Mutex::new(Vec::new()));
    let seen = opened.clone();
    let browse: Arc<mail_app::ui::native::AddAccountBrowse> = Arc::new(move |url| {
        seen.lock().unwrap().push(url.to_owned());
        Ok(())
    });
    let fits = Arc::new(Mutex::new(Vec::new()));
    let asked = fits.clone();
    let fit: Arc<mail_app::ui::native::AddAccountFit> =
        Arc::new(move |extent| asked.lock().unwrap().push(extent));
    let wiring = AddAccountWiring::new(seams(&script, &log), browse).fitting(fit);
    let config = HarnessConfig::new(view)
        .with_clock(Clock::Virtual)
        .with_context(store)
        .with_context(environment(theme))
        .with_context(AddAccountOpened::new(wiring, prefill.map(str::to_owned)));
    let config = match icons {
        Some(icons) => config.with_context(icons),
        None => config,
    };
    let mut harness = Harness::new(add_account_root, config);
    harness.advance(Duration::from_millis(300));
    Window {
        harness,
        fits,
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

/// The pixel size of the PNG behind the `data:` URI in an inline style's `url("...")`.
fn png_size_in(style: &str) -> (u32, u32) {
    use base64::Engine as _;
    let at = style
        .find("data:image/png;base64,")
        .unwrap_or_else(|| panic!("no png data uri in {style:.80}"));
    let data = style[at + "data:image/png;base64,".len()..]
        .split('"')
        .next()
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    let image = image::load_from_memory(&bytes).unwrap();
    (image.width(), image.height())
}

#[test]
fn a_provider_icon_is_a_picture_as_many_pixels_across_as_the_screen_draws_it() {
    // At scale 2 the list's 28 px favicon covers 56 device pixels and a step's 48 px header 96:
    // each is handed a picture exactly that size, not one the renderer stretches and softens.
    let icons = tempfile::tempdir().unwrap();
    let cached = image::RgbaImage::from_pixel(96, 96, image::Rgba([0x2a, 0x5d, 0xb0, 0xff]));
    image::DynamicImage::ImageRgba8(cached)
        .save(icons.path().join("fastmail.png"))
        .unwrap();
    let view = Viewport {
        scale_percent: 200,
        ..VIEW
    };
    let loaded = mail_app::ui::native::read_icons(icons.path());
    let mut w = open_with(
        Script::default(),
        None,
        ds::prelude::Theme::Light,
        view,
        Some(loaded),
    );
    on_step(&mut w, "providers");
    let icon = ".ds-ext-icon[*|data-kind=image]";
    let check = |w: &Window, place: &str| {
        let rect = w
            .harness
            .rect(icon)
            .unwrap_or_else(|| panic!("{place}: no favicon is drawn:\n{}", w.harness.html()));
        let style = w.harness.attr(icon, "style").unwrap();
        let drawn = (
            (rect.size.width.0 * 2.0).round() as u32,
            (rect.size.height.0 * 2.0).round() as u32,
        );
        assert_eq!(png_size_in(&style), drawn, "{place}");
    };
    check(&w, "the list");
    type_text(&mut w, "fast");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    check(&w, "the sign-in header");
}

/// The window opens on the list of providers with a title of its own, looking nothing up; then
/// the sign-in form. Both pass the markup lint.
#[test]
fn the_window_opens_on_the_list_of_providers_and_passes_the_lint() {
    let mut w = open(Script::default(), None);
    on_step(&mut w, "providers");
    let providers = w.harness.html();
    assert!(providers.contains("Add Account"), "{providers}");
    for name in [
        "Google",
        "Microsoft",
        "Fastmail",
        "iCloud",
        "Yahoo",
        "GMX",
        "Other",
    ] {
        assert!(
            providers.contains(name),
            "{name} is not offered:\n{providers}"
        );
    }
    assert!(
        !providers.contains("Email (IMAP)"),
        "the generic row is the list's Other…"
    );
    assert!(
        w.log.lookups.lock().unwrap().is_empty(),
        "the list looked something up"
    );

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
        assert!(offences.is_empty(), "the lint on {name}: {offences:#?}");
    }
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
        h.html().contains("Outgoing server") || h.html().contains("Server web address")
    });
}

/// Choose the `nth` (from 1) of the protocol's options in its pop-up.
fn pick_protocol(w: &mut Window, nth: usize) {
    click_text(w, ".ds-popup .ds-button");
    until(w, "the protocol's menu never opened", |h| {
        h.count(".ds-menu") == 1
    });
    click_text(w, &format!(".ds-menu .ds-menu-item:nth-child({nth})"));
    w.harness.advance(Duration::from_millis(200));
}

/// Return, from the server field: after a pick the keyboard is on nothing, and Return belongs to
/// whatever has it.
fn continue_from_server(w: &mut Window) {
    click_text(w, "input[aria-label=\"Server\"]");
    w.harness.key(Key::Enter);
}

fn no_servers() -> Script {
    Script {
        found: false,
        ..Script::default()
    }
}

#[test]
fn a_whole_typed_pop3_add_through_the_window() {
    let mut w = open(no_servers(), None);
    to_server_form(&mut w);
    let html = w.harness.html();
    for word in [
        "Incoming",
        "Outgoing",
        "Sign in",
        "Server type",
        "(IMAP)",
        "(SSL/TLS)",
    ] {
        assert!(html.contains(word), "{word}:\n{html}");
    }
    assert!(!html.contains(SECRET), "the password reached the markup");
    assert_eq!(w.log.lookups.lock().unwrap().len(), 1);

    // POP3: the form changes at once (no sending): the guessed host and the ports follow.
    pick_protocol(&mut w, 2);
    let html = w.harness.html();
    assert!(html.contains("Older servers (POP)"), "{html}");
    assert!(html.contains("pop.example.test"), "{html}");
    assert!(html.contains("995"), "{html}");
    assert_eq!(
        w.log.lookups.lock().unwrap().len(),
        1,
        "a pick looks nothing up"
    );

    continue_from_server(&mut w);
    on_step(&mut w, "review");
    assert!(w.log.adds.lock().unwrap().is_empty());
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
}

/// The form of typed servers, for IMAP and then for JMAP, which drops the outgoing part and asks
/// a session URL and a token instead. Both pass the markup lint.
#[test]
fn the_typed_servers_form_drops_the_outgoing_part_for_jmap_and_passes_the_lint() {
    let mut w = open(no_servers(), None);
    to_server_form(&mut w);
    let imap = w.harness.html();
    pick_protocol(&mut w, 3);
    let jmap = w.harness.html();
    assert!(jmap.contains("Server web address"), "JMAP: {jmap}");
    assert!(jmap.contains("Access token"), "JMAP: {jmap}");
    assert!(!jmap.contains("Outgoing server"), "JMAP: {jmap}");
    assert!(
        !jmap.contains("Outgoing"),
        "JMAP has an outgoing part:\n{jmap}"
    );
    let css = format!(
        "{}\n{}",
        ds_shell::stylesheet(),
        include_str!("../src/ui/style/accounts.css")
    );
    let config = ds_lint::LintConfig::new(&ds_shell::kits());
    for (name, html) in [("imap", imap), ("jmap", jmap)] {
        let offences = ds_lint::markup(&html, &css, &config);
        assert!(offences.is_empty(), "the lint on {name}: {offences:#?}");
    }
}

#[test]
fn a_wrong_port_is_not_sent_and_is_marked_on_its_field() {
    let mut w = open(no_servers(), None);
    to_server_form(&mut w);
    // Protocol, server, security, port: the keyboard walks the form in order.
    for _ in 0..4 {
        w.harness.key(Key::Tab);
    }
    type_text(&mut w, "x");
    w.harness.key(Key::Enter);
    w.harness.advance(Duration::from_millis(300));
    let html = w.harness.html();
    assert!(html.contains("is not valid"), "the port is marked:\n{html}");
    assert_eq!(step(&w).as_deref(), Some("sign-in"), "nothing was sent");
    assert!(w.log.adds.lock().unwrap().is_empty());
}

/// What the window asked for, each size once in a row.
fn asked(w: &Window) -> Vec<Extent> {
    let mut sizes: Vec<Extent> = Vec::new();
    for size in w.fits.lock().unwrap().iter() {
        if sizes.last() != Some(size) {
            sizes.push(*size);
        }
    }
    sizes
}

#[test]
fn the_window_asks_for_the_size_of_each_step() {
    let mut w = open(no_servers(), None);
    on_step(&mut w, "providers");
    let list = asked(&w);
    assert_eq!(list.len(), 1, "{list:?}");
    type_text(&mut w, "zz");
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    let short = *asked(&w).last().unwrap();
    assert_ne!(short, list[0], "the short form is not the list's size");
    assert!(short.height < list[0].height);
    type_text(&mut w, "ada@example.test");
    w.harness.key(Key::Tab);
    type_text(&mut w, SECRET);
    w.harness.key(Key::Enter);
    until(&mut w, "the form of typed servers never came", |h| {
        h.html().contains("Outgoing server")
    });
    let server = *asked(&w).last().unwrap();
    assert!(
        server.height > short.height && server.width > short.width,
        "{server:?}"
    );
    pick_protocol(&mut w, 3);
    let jmap = *asked(&w).last().unwrap();
    assert!(
        jmap.height < server.height,
        "JMAP has no outgoing part: {jmap:?}"
    );
    pick_protocol(&mut w, 1);
    assert_eq!(*asked(&w).last().unwrap(), server, "and back");
    continue_from_server(&mut w);
    on_step(&mut w, "review");
    let review = *asked(&w).last().unwrap();
    assert!(review.height < server.height, "{review:?}");
    assert_eq!(review.width, list[0].width);
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

/// Drive a fresh window, in a viewport of `view`'s size, to the step `target` names, and call
/// `there` with it once it is there (for the working step, while the lookup is still held).
fn drive_to(
    target: &str,
    theme: ds::prelude::Theme,
    view: Viewport,
    there: &mut dyn FnMut(&mut Window),
) {
    let script = match target {
        "failed" => Script {
            add: Err("cannot save the account".to_owned()),
            ..Script::default()
        },
        "providers" | "sign-in" | "review" | "working" | "browser" => Script::default(),
        _ => no_servers(),
    };
    let hold = script.hold.clone();
    let mut w = open_sized(script, None, theme, view);
    on_step(&mut w, "providers");
    if target == "providers" {
        return there(&mut w);
    }
    if target == "browser" {
        type_text(&mut w, "google");
        w.harness.key(Key::Enter);
        on_step(&mut w, "sign-in");
        type_text(&mut w, "ada@gmail.com");
        w.harness.key(Key::Enter);
        on_step(&mut w, "browser");
        return there(&mut w);
    }
    // Everything else goes through "Other…" for a typed server, or Fastmail for a password.
    let typed_servers = target.starts_with("server");
    type_text(&mut w, if typed_servers { "zz" } else { "fast" });
    w.harness.key(Key::Enter);
    on_step(&mut w, "sign-in");
    type_text(&mut w, "ada@example.test");
    if target == "sign-in" {
        return there(&mut w);
    }
    w.harness.key(Key::Tab);
    type_text(&mut w, SECRET);
    let held = hold.lock().unwrap();
    w.harness.key(Key::Enter);
    if target == "working" {
        on_step(&mut w, "working");
        there(&mut w);
        drop(held);
        return;
    }
    drop(held);
    if typed_servers {
        until(&mut w, "the typed servers' form", |h| {
            h.html().contains("Outgoing server")
        });
        match target {
            "server-pop3" => pick_protocol(&mut w, 2),
            "server-jmap" => pick_protocol(&mut w, 3),
            _ => {}
        }
        return there(&mut w);
    }
    on_step(&mut w, "review");
    if target == "failed" {
        w.harness.key(Key::Enter);
        on_step(&mut w, "failed");
    }
    there(&mut w)
}

/// The typed servers' form for each protocol, and every step in a window of the size it asks
/// for, light and dark: set `MAILO_SHOTS` to the directory and run with `--ignored`. Each
/// step is driven twice: once in a viewport larger than any step, to read what it asks, and
/// once in a viewport of exactly that size, which is what is painted.
#[test]
#[ignore = "picture generator: set MAILO_SHOTS to a directory and run with --ignored"]
fn render_the_typed_forms_and_the_resized_steps() {
    let dir = std::path::PathBuf::from(
        std::env::var("MAILO_SHOTS").expect("MAILO_SHOTS names the directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let large = Viewport {
        width: 900,
        height: 1000,
        scale_percent: 100,
    };
    for (name, theme) in [
        ("light", ds::prelude::Theme::Light),
        ("dark", ds::prelude::Theme::Dark),
    ] {
        for target in [
            "providers",
            "sign-in",
            "server-imap",
            "server-pop3",
            "server-jmap",
            "working",
            "review",
            "failed",
            "browser",
        ] {
            let mut asked_size = None;
            drive_to(target, theme, large, &mut |w| {
                asked_size = asked(w).last().copied();
            });
            let asked = asked_size.expect("the step asked for a size");
            let view = Viewport {
                width: asked.width,
                height: asked.height,
                scale_percent: 100,
            };
            let shot = |prefix: &str, view: Viewport| {
                drive_to(target, theme, view, &mut |w| {
                    w.harness.advance(Duration::from_millis(600));
                    w.harness
                        .render()
                        .unwrap()
                        .save(dir.join(format!("{prefix}-{target}-{name}.png")))
                        .unwrap();
                });
            };
            shot("sized", view);
            println!("{target} {name}: {}x{}", asked.width, asked.height);
            if target.starts_with("server") {
                // The same form in the window as E5 opened it.
                shot("fixed", VIEW);
            }
        }
    }
}
