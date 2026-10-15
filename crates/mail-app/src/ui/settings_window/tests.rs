//! The Settings window, over the reference fixture and a temporary config directory: ⌘, and the
//! foot menu ask for it, ⌘K's entries ask for it on their page, each schema key is a row whose control
//! writes `settings.toml`, and every page draws in the window. An account's page in the Accounts
//! pane is `account`'s (`account/tests.rs`).

use super::{OpenSettings, SettingsAt, SettingsWindows, settings_root};
use crate::settings::{BrandLogos, LoadRemoteImages, MailSettings, Spelling};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, Work, chord, click, dispatching, drain_seen, rebuild_into, work,
};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::{self, VirtualDom};
use std::sync::{Arc, Mutex};

/// The Settings window over `built`, on `page`.
pub(super) fn opened_on(built: &Work, page: SettingsPage) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    match page {
        SettingsPage::General => (dom, seen),
        other => {
            let seen =
                click(&mut dom, seen.one("aria-label", other.name())).merge(drain_seen(&mut dom));
            (dom, seen)
        }
    }
}

/// The Settings window over `built`, on General's trusted senders, pushed from their row.
fn trusted_senders(built: &Work) -> (VirtualDom, Seen) {
    let (mut dom, seen) = opened_on(built, SettingsPage::General);
    let seen =
        click(&mut dom, seen.one("id", "general-pane-open-trusted")).merge(drain_seen(&mut dom));
    (dom, seen)
}

/// What `settings.toml` in the fixture's config directory holds now.
fn stored(built: &Work) -> MailSettings {
    crate::settings::store(crate::settings::root_for(&built.dirs.config))
        .load::<MailSettings>()
        .value
}

/// The windows the main window asked for, each with where it asked it to be.
#[derive(Default)]
pub(in crate::ui) struct Asked(Mutex<Vec<Option<SettingsAt>>>);

impl Asked {
    /// Every ask so far, in order.
    pub(in crate::ui) fn asks(&self) -> Vec<Option<SettingsAt>> {
        self.0.lock().map(|asks| asks.clone()).unwrap_or_default()
    }
}

impl OpenSettings for Asked {
    fn open(&self, at: Option<SettingsAt>) {
        if let Ok(mut asks) = self.0.lock() {
            asks.push(at);
        }
    }
}

/// The main window over `built`, its Settings window a recorder.
fn main_window(built: &Work) -> (VirtualDom, Seen, Arc<Asked>) {
    dispatching();
    let asked = Arc::new(Asked::default());
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(SettingsWindows(asked.clone()));
    let seen = rebuild_into(&mut dom);
    (dom, seen, asked)
}

#[tokio::test]
async fn command_comma_and_the_foot_menu_ask_for_the_settings_window() {
    let built = work();
    let (mut dom, seen, asked) = main_window(&built);
    let _ = chord(
        &mut dom,
        ",",
        crate::ui::fixtures::PRIMARY,
        dioxus_core::ElementId(INSIDE_THE_SHELL as usize),
    );
    assert_eq!(asked.asks(), [None], "⌘, asked for nothing");
    let opened = click(
        &mut dom,
        seen.one("aria-label", crate::ui::copy::aria::SIDEBAR_MENU),
    );
    let opened = crate::ui::fixtures::settle(&mut dom, opened);
    crate::ui::fixtures::pick_named(&mut dom, &opened, "Settings\u{2026}").await;
    assert_eq!(
        asked.asks(),
        [None, None],
        "the menu's Settings asked for nothing"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("data-page=\"General\""),
        "Settings was drawn in the main window: {page}"
    );
}

#[tokio::test]
async fn general_draws_reading_then_mail_with_their_rows() {
    let built = work();
    let (dom, _seen) = opened_on(&built, SettingsPage::General);
    let page = dioxus_ssr::render(&dom);
    let mut from = 0;
    for label in [
        "Reading",
        "Load remote images",
        "Trusted Senders",
        "Brand logos",
        "Mail",
        "Notifications",
        "Check spelling",
        "Search server",
    ] {
        let at = page[from..]
            .find(&format!(">{label}<"))
            .unwrap_or_else(|| panic!("no {label:?} after the rows before it: {page}"));
        from += at;
    }
    assert!(!page.contains("Mail list"), "{page}");
    assert!(page.contains("data-page=\"General\""), "{page}");
}

/// Every key of the schema is a row of General, but the two left on purpose: the provider marks
/// are detent's to set, and the trusted senders are a list with a page of their own.
#[test]
fn every_schema_key_is_on_general_or_left_to_detent_on_purpose() {
    let named: Vec<&str> = super::general::SECTIONS
        .iter()
        .flat_map(|section| section.lines.iter())
        .filter_map(|line| match line {
            super::general::Line::Key(path) => Some(*path),
            super::general::Line::TrustedSenders => None,
        })
        .collect();
    let left = ["window.provider_marks", "reading.trusted_image_senders"];
    for key in crate::settings::schema().key {
        let path = key.path.0.as_str();
        assert!(
            named.contains(&path) != left.contains(&path),
            "{path} is neither on General nor left to detent"
        );
    }
    for path in named {
        assert!(
            crate::settings::schema()
                .key
                .iter()
                .any(|key| key.path.0 == path),
            "General names {path}, which the schema has not"
        );
    }
}

/// The General page's controls each write `settings.toml`: two switches, then a choice.
#[tokio::test]
async fn general_controls_write_settings_toml() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::General);
    assert_eq!(
        stored(&built).reading.brand_logos,
        BrandLogos::Off,
        "before: brand logos"
    );
    // A row and its switch both carry the key's label: the switch is the checkable one in it.
    click(
        &mut dom,
        seen.after("aria-label", "Brand logos", "aria-checked")[0],
    );
    assert_eq!(
        stored(&built).reading.brand_logos,
        BrandLogos::On,
        "the brand logos switch"
    );
    click(
        &mut dom,
        seen.after("aria-label", "Check spelling", "aria-checked")[0],
    );
    assert_eq!(
        stored(&built).compose.spelling,
        Spelling::Off,
        "the check spelling switch"
    );
}

/// Each page, by the sidebar's row, and a line only that page draws.
const PAGES: &[(SettingsPage, &str)] = &[
    (SettingsPage::General, "Check spelling"),
    (SettingsPage::Accounts, "Add Account\u{2026}"),
    (SettingsPage::Rules, "Vacation Reply"),
    (SettingsPage::Keyboard, "Change the key for"),
];

#[test]
fn settings_has_4_pages_in_that_order() {
    assert_eq!(
        SettingsPage::ALL.map(SettingsPage::name),
        ["General", "Accounts", "Rules", "Keyboard"]
    );
}

#[tokio::test]
async fn every_page_is_drawn_in_the_window_from_its_sidebar_row() {
    let built = work();
    assert_eq!(
        PAGES.iter().map(|(page, _)| *page).collect::<Vec<_>>(),
        SettingsPage::ALL,
        "a page has no case here"
    );
    for (page, drawn) in PAGES {
        let (dom, _seen) = opened_on(&built, *page);
        let markup = dioxus_ssr::render(&dom);
        assert!(
            markup.contains(&format!("data-page=\"{}\"", page.name())),
            "{page:?} is not the page shown: {markup}"
        );
        assert!(
            markup.contains(drawn),
            "{page:?} drew no {drawn:?}: {markup}"
        );
        let offences = crate::ui::style::tests::markup_offences(&markup);
        assert!(offences.is_empty(), "{page:?}: {offences:#?}");
    }
}

#[tokio::test]
async fn an_open_window_turns_to_the_page_asked_for() {
    let built = work();
    let asked = super::SettingsAsked::default();
    asked.ask(SettingsAt::Page(SettingsPage::Rules));
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(asked.clone());
    let _ = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("data-page=\"Rules\""),
        "it opened elsewhere: {page}"
    );

    asked.ask(SettingsAt::Page(SettingsPage::Keyboard));
    for _ in 0..20 {
        let quiet = std::time::Duration::from_millis(50);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("data-page=\"Keyboard\""),
        "it did not turn: {page}"
    );
}

#[tokio::test]
async fn each_trusted_sender_is_a_row_whose_remove_stops_its_images() {
    let built = work();
    let mut settings = MailSettings::default();
    settings.reading.remote_images = LoadRemoteImages::Trusted;
    settings.reading.trusted_image_senders = vec![
        "news@example.test".to_owned(),
        "bob@example.test".to_owned(),
    ];
    crate::settings::store(crate::settings::root_for(&built.dirs.config))
        .save(&settings)
        .unwrap_or_else(|why| panic!("{why:?}"));
    let (mut dom, seen) = trusted_senders(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("news@example.test"), "{page}");
    assert!(
        !page.contains("[\"news@example.test\""),
        "the list is drawn as its raw value too: {page}"
    );
    click(
        &mut dom,
        seen.one(
            "aria-label",
            &crate::ui::copy::aria::stop_loading_images("news@example.test"),
        ),
    );
    assert_eq!(
        stored(&built).reading.trusted_image_senders,
        ["bob@example.test"]
    );
}

#[tokio::test]
async fn trusting_senders_with_none_yet_says_how_one_is_trusted() {
    let built = work();
    let mut settings = MailSettings::default();
    settings.reading.remote_images = LoadRemoteImages::Trusted;
    crate::settings::store(crate::settings::root_for(&built.dirs.config))
        .save(&settings)
        .unwrap_or_else(|why| panic!("{why:?}"));
    let (dom, _) = trusted_senders(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(crate::ui::copy::empty::NO_TRUSTED_SENDERS),
        "{page}"
    );
    assert!(
        page.contains(crate::ui::copy::fields::HOW_TO_TRUST),
        "{page}"
    );
}
