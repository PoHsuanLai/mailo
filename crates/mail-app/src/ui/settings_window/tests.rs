//! The Settings window, over the reference fixture and a temporary config directory: ⌘, and the
//! gear ask for it, ⌘K's entries ask for it on their page, each schema key is a row whose control
//! writes `settings.toml`, every page draws in the window, and the Accounts page opens an
//! account's own sheet in the window.

use super::{OpenSettings, SettingsWindows, settings_root};
use crate::settings::{BrandLogos, MailSettings, ProviderMarks, Spelling};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, Work, chord, click, dispatching, drain_seen, press, rebuild_into, work,
};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::{self, VirtualDom};
use dioxus::prelude::Modifiers;
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

/// What `settings.toml` in the fixture's config directory holds now.
fn stored(built: &Work) -> MailSettings {
    crate::settings::store(crate::settings::root_for(&built.dirs.config))
        .load::<MailSettings>()
        .value
}

/// The windows the main window asked for, each with the page it asked it to show.
#[derive(Default)]
pub(in crate::ui) struct Asked(Mutex<Vec<Option<SettingsPage>>>);

impl Asked {
    /// Every ask so far, in order.
    pub(in crate::ui) fn asks(&self) -> Vec<Option<SettingsPage>> {
        self.0.lock().map(|asks| asks.clone()).unwrap_or_default()
    }
}

impl OpenSettings for Asked {
    fn open(&self, page: Option<SettingsPage>) {
        if let Ok(mut asks) = self.0.lock() {
            asks.push(page);
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
async fn command_comma_and_the_gear_ask_for_the_settings_window() {
    let built = work();
    let (mut dom, seen, asked) = main_window(&built);
    let _ = chord(
        &mut dom,
        ",",
        Modifiers::CONTROL,
        dioxus_core::ElementId(INSIDE_THE_SHELL as usize),
    );
    assert_eq!(asked.asks(), [None], "⌘, asked for nothing");
    click(&mut dom, seen.one("aria-label", "Settings"));
    assert_eq!(asked.asks(), [None, None], "the gear asked for nothing");
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("data-page=\"General\""),
        "Settings was drawn in the main window: {page}"
    );
}

#[tokio::test]
async fn the_window_has_a_row_per_key() {
    let built = work();
    let (dom, _seen) = opened_on(&built, SettingsPage::General);
    let page = dioxus_ssr::render(&dom);
    for label in [
        "Provider marks",
        "New mail",
        "Check spelling",
        "Brand logos",
        "Search the server automatically",
    ] {
        assert!(page.contains(label), "no {label:?} in Settings: {page}");
    }
    assert!(page.contains("data-page=\"General\""), "{page}");
}

#[tokio::test]
async fn a_switch_writes_settings_toml() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::General);
    assert_eq!(stored(&built).reading.brand_logos, BrandLogos::Off);
    // A row and its switch both carry the key's label: the switch is the checkable one in it.
    click(
        &mut dom,
        seen.after("aria-label", "Brand logos", "aria-checked")[0],
    );
    assert_eq!(stored(&built).reading.brand_logos, BrandLogos::On);
    click(
        &mut dom,
        seen.after("aria-label", "Check spelling", "aria-checked")[0],
    );
    assert_eq!(stored(&built).compose.spelling, Spelling::Off);
}

#[tokio::test]
async fn provider_marks_are_a_choice() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::General);
    // Icons, then Letters: the segments after the control's group.
    let segments = seen.after("aria-label", "Provider marks", "aria-checked");
    click(&mut dom, segments[1]);
    let _ = drain_seen(&mut dom);
    assert_eq!(stored(&built).window.provider_marks, ProviderMarks::Letters);
}

#[tokio::test]
async fn the_accounts_page_opens_an_account_s_sheet_in_the_window() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::Accounts);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("poh@acme.example"), "{page}");
    assert!(page.contains("Add Account\u{2026}"), "{page}");

    click(
        &mut dom,
        seen.one("aria-label", "Details for poh@acme.example"),
    );
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Account Settings"),
        "no account sheet: {page}"
    );

    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("Account Settings"),
        "Escape left the sheet open: {page}"
    );
    assert!(page.contains("data-page=\"Accounts\""), "{page}");
}

/// Each page, by the sidebar's row, and a line only that page draws.
const PAGES: &[(SettingsPage, &str)] = &[
    (SettingsPage::General, "Check spelling"),
    (SettingsPage::Accounts, "Add Account\u{2026}"),
    (SettingsPage::Contacts, "Import vCard\u{2026}"),
    (SettingsPage::Rules, "Vacation reply"),
    (SettingsPage::Keys, "S/MIME"),
    (SettingsPage::Keyboard, "Change the key for"),
];

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
    asked.ask(SettingsPage::Rules);
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

    asked.ask(SettingsPage::Keyboard);
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
