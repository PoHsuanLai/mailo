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
    crate::ui::fixtures::pick_named(&mut dom, &opened, crate::ui::copy::menus::SETTINGS).await;
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
        crate::ui::copy::headers::READING,
        "Load remote images",
        crate::ui::copy::titles::TRUSTED_SENDERS,
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
        .chain(super::general::PRIVACY.iter())
        .flat_map(|section| section.lines.iter())
        .filter_map(|line| match line {
            super::general::Line::Key(path) => Some(*path),
            super::general::Line::TrustedSenders | super::general::Line::DefaultMail => None,
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
    (
        SettingsPage::Accounts,
        crate::ui::copy::controls::ADD_ACCOUNT,
    ),
    (SettingsPage::Spaces, crate::ui::copy::controls::NEW_SPACE),
    (SettingsPage::Rules, crate::ui::copy::controls::NEW_RULE),
    (SettingsPage::Vacation, "id=\"vacation-pane\""),
    (SettingsPage::Keyboard, "Change the key for"),
    (SettingsPage::Privacy, "Protect mail activity"),
];

#[test]
fn settings_has_7_pages_in_that_order() {
    assert_eq!(
        SettingsPage::ALL.map(SettingsPage::name),
        [
            crate::ui::copy::titles::GENERAL,
            crate::ui::copy::titles::ACCOUNTS,
            crate::ui::copy::titles::SPACES,
            crate::ui::copy::titles::RULES,
            "Vacation Reply",
            crate::ui::copy::titles::KEYBOARD,
            crate::ui::copy::titles::PRIVACY
        ]
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

/// Vacation Reply is a pane as Spaces is: its root lists the accounts, each saying whether its
/// reply is on, and a row pushes that account's reply, switch and all, as a page of its own.
#[tokio::test]
async fn vacation_reply_lists_the_accounts_and_a_row_pushes_its_reply() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, SettingsPage::Vacation);
    let list = dioxus_ssr::render(&dom);
    let rows: Vec<_> = crate::ui::data::account_rows(&built.store)
        .into_iter()
        .filter(|row| !row.is_local())
        .collect();
    let first = rows
        .first()
        .expect("the fixture has an account with a server");
    for row in &rows {
        assert!(
            list.contains(&format!("id=\"vacation-pane-open-{}\"", row.id)),
            "{} has no row: {list}",
            row.address
        );
    }
    assert!(
        !list.contains("role=\"switch\""),
        "the root draws a reply: {list}"
    );

    let pushed = click(
        &mut dom,
        seen.one("id", &format!("vacation-pane-open-{}", first.id)),
    )
    .merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("role=\"switch\""),
        "the reply's page has no switch: {page}"
    );
    assert!(
        page.contains(&first.shown()),
        "the page is not titled by its account: {page}"
    );
    let back = pushed.one("id", "vacation-pane-back-detail");
    let _ = click(&mut dom, back);
    // The page slides away before it is gone.
    let _ = crate::ui::fixtures::later(&mut dom).await;
    assert!(
        !dioxus_ssr::render(&dom).contains("role=\"switch\""),
        "back did not go back"
    );
}

/// Each page that lists things, the add row under them, and the label of the list it stands in.
const ADDS: &[(SettingsPage, &str, &str)] = &[
    (
        SettingsPage::Accounts,
        crate::ui::copy::controls::ADD_ACCOUNT,
        crate::ui::copy::aria::ADD_ACCOUNT,
    ),
    (
        SettingsPage::Spaces,
        crate::ui::copy::controls::NEW_SPACE,
        crate::ui::copy::aria::ADD_A_SPACE,
    ),
    (
        SettingsPage::Rules,
        crate::ui::copy::controls::NEW_RULE,
        crate::ui::copy::aria::ADD_A_RULE,
    ),
];

/// Accounts, Spaces and Rules set their add row alike: in a list of its own under the things it
/// adds to, led by the plain plus and not a coloured tile.
#[tokio::test]
async fn every_add_row_stands_in_a_list_of_its_own_led_by_the_plain_plus() {
    let built = work();
    for (page, row, list) in ADDS {
        let (dom, _seen) = opened_on(&built, *page);
        let markup = dioxus_ssr::render(&dom);
        let at = markup
            .find(&format!("aria-label=\"{list}\" data-style=\"grouped\""))
            .unwrap_or_else(|| panic!("{page:?} has no {list:?} list: {markup}"));
        let added = &markup[at..];
        let end = added[1..]
            .find("class=\"ds-list\"")
            .map_or(added.len(), |end| end + 1);
        let added = &added[..end];
        assert!(
            added.contains(row),
            "{page:?}: {row:?} is not in {list:?}: {added}"
        );
        assert_eq!(
            added.matches("role=\"option\"").count(),
            1,
            "{page:?}: {added}"
        );
        assert!(added.contains("class=\"add-mark\""), "{page:?}: {added}");
        assert!(
            !added.contains("ds-tile"),
            "{page:?} leads with a tile: {added}"
        );
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
async fn trusting_senders_with_none_yet_says_there_are_none() {
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
}
