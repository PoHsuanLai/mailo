//! Settings in the running window, over the reference fixture and a temporary config directory:
//! ⌘, opens it, each schema key is a row whose control writes `settings.toml`, the Accounts page
//! opens an account's own sheet, and Escape closes it.

use crate::settings::{BrandLogos, MailSettings, ProviderMarks, Spelling};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, Work, chord, click, dispatching, drain_seen, press, rebuild_into, work,
};
use crate::ui::view::SettingsPage;
use dioxus::dioxus_core::{self, VirtualDom};
use dioxus::prelude::Modifiers;

/// The window over `built`, Settings open on `page`.
pub(super) fn opened_on(built: &Work, page: SettingsPage) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let _ = rebuild_into(&mut dom);
    let seen = chord(
        &mut dom,
        ",",
        Modifiers::CONTROL,
        dioxus_core::ElementId(INSIDE_THE_SHELL as usize),
    )
    .merge(drain_seen(&mut dom));
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

#[tokio::test]
async fn command_comma_opens_settings_with_a_row_per_key() {
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
async fn a_switch_writes_settings_toml_and_the_window_follows() {
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
async fn provider_marks_are_a_choice_and_the_chips_follow() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::General);
    // Icons, then Letters: the segments after the control's group.
    let segments = seen.after("aria-label", "Provider marks", "aria-checked");
    click(&mut dom, segments[1]);
    let _ = drain_seen(&mut dom);
    assert_eq!(stored(&built).window.provider_marks, ProviderMarks::Letters);
}

#[tokio::test]
async fn the_accounts_page_opens_an_account_s_sheet_and_settings_steps_aside() {
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
    assert!(
        !page.contains("data-page=\"Accounts\""),
        "Settings is still over it: {page}"
    );

    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("data-page=\"Accounts\""),
        "Settings did not come back: {page}"
    );
}

#[tokio::test]
async fn escape_closes_settings() {
    let built = work();
    let (mut dom, _seen) = opened_on(&built, SettingsPage::General);
    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains("data-page=\"General\""), "{page}");
}

#[tokio::test]
async fn the_gear_in_the_sidebar_s_foot_opens_settings() {
    let built = work();
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    click(&mut dom, seen.one("aria-label", "Settings"));
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("data-page=\"General\""),
        "the gear opened something else: {page}"
    );
    assert!(!page.contains("aria-label=\"Space editor\""), "{page}");
}
