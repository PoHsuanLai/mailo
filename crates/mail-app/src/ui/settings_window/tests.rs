//! The Settings window, over the reference fixture and a temporary config directory: ⌘, and the
//! gear ask for it, each schema key is a row whose control writes `settings.toml`, and the
//! Accounts page opens an account's own sheet in the window.

use super::{OpenSettings, SettingsWindows, settings_root};
use crate::settings::{BrandLogos, MailSettings, ProviderMarks, Spelling};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, Work, chord, click, dispatching, drain_seen, press, rebuild_into, work,
};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::{self, VirtualDom};
use dioxus::prelude::Modifiers;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The Settings window over `built`, on `page`.
pub(super) fn opened_on(built: &Work, page: SettingsPage) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    match page {
        SettingsPage::General => (dom, seen),
        SettingsPage::Accounts => {
            let seen =
                click(&mut dom, seen.one("aria-label", "Accounts")).merge(drain_seen(&mut dom));
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

/// Counts the windows the main window asked for.
#[derive(Default)]
struct Asked(AtomicUsize);

impl OpenSettings for Asked {
    fn open(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
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
    assert_eq!(asked.0.load(Ordering::SeqCst), 1, "⌘, asked for nothing");
    click(&mut dom, seen.one("aria-label", "Settings"));
    assert_eq!(
        asked.0.load(Ordering::SeqCst),
        2,
        "the gear asked for nothing"
    );
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
        "Contacts\u{2026}",
        "Keyboard Shortcuts\u{2026}",
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
