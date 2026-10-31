//! The Settings window, over the reference fixture and a temporary config directory: ⌘, and the
//! foot menu ask for it, ⌘K's entries ask for it on their page, each schema key is a row whose control
//! writes `settings.toml`, and every page draws in the window. An account's page in the Accounts
//! pane is `account`'s (`account/tests.rs`).

use super::{OpenSettings, SettingsAt, SettingsWindows, settings_root};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, Work, chord, click, dispatching, drain_seen, rebuild_into, work,
};
use crate::ui::view::PageId;
use dioxus::dioxus_core::{self, VirtualDom};
use mail_ui_model::settings::{BrandLogos, LoadRemoteImages, MailSettings, Spelling};
use std::sync::{Arc, Mutex};

/// The Settings window over `built`, on `page`.
pub(in crate::ui) fn opened_on(built: &Work, page: PageId) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    if page == PageId::GENERAL {
        return (dom, seen);
    }
    let label = page_of(page).name();
    let seen = click(&mut dom, seen.one("aria-label", label)).merge(drain_seen(&mut dom));
    (dom, seen)
}

/// The page `id` names.
fn page_of(id: PageId) -> &'static dyn super::page::Page {
    super::PAGES
        .iter()
        .copied()
        .find(|page| page.id() == id)
        .unwrap_or_else(|| panic!("{id:?} is not in PAGES"))
}

/// The Settings window over `built`, on Privacy's trusted senders, pushed from their row.
fn trusted_senders(built: &Work) -> (VirtualDom, Seen) {
    let (mut dom, seen) = opened_on(built, PageId::PRIVACY);
    let seen =
        click(&mut dom, seen.one("id", "privacy-pane-open-trusted")).merge(drain_seen(&mut dom));
    (dom, seen)
}

/// What `settings.toml` in the fixture's config directory holds now.
fn stored(built: &Work) -> MailSettings {
    mail_ui_model::settings::store(mail_ui_model::settings::root_for(&built.dirs.config))
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
        .with_root_context(crate::ui::fixtures::client(&built.store))
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
        seen.one("aria-label", mail_ui_model::copy::aria::SIDEBAR_MENU),
    );
    let opened = crate::ui::fixtures::settle(&mut dom, opened);
    crate::ui::fixtures::pick_named(&mut dom, &opened, mail_ui_model::copy::menus::SETTINGS).await;
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

/// Each page that draws the schema, with the rows it draws in the order drawn.
const KEYED: &[(PageId, &[&str])] = &[
    (
        PageId::GENERAL,
        &[mail_ui_model::copy::headers::MAIL, "Search server"],
    ),
    (
        PageId::COMPOSING,
        &[
            "Message format",
            "Reply in the same format as the original",
            "Undo send",
            "Check spelling",
        ],
    ),
    (
        PageId::VIEWING,
        &[
            "Brand logos",
            "Prefer plain text",
            "Mark as read",
            "Message text size",
            mail_ui_model::copy::headers::MESSAGE_LIST,
        ],
    ),
    (
        PageId::NOTIFICATIONS,
        &[
            "Notifications",
            "Notify for",
            mail_ui_model::copy::titles::VIPS,
            "Notify in",
            "Sound",
            "Badge counts",
            mail_ui_model::copy::headers::NOTIFY_FOR_ACCOUNTS,
        ],
    ),
    (
        PageId::PRIVACY,
        &[
            "Load remote images",
            mail_ui_model::copy::titles::TRUSTED_SENDERS,
            mail_ui_model::copy::headers::REMOTE_CONTENT,
            "Protect mail activity",
        ],
    ),
    (
        PageId::JUNK_MAIL,
        &[
            "Filter Junk Mail",
            "When Mail Is Junk",
            mail_ui_model::copy::headers::NEVER_JUNK_MAIL_FROM,
            "Contacts",
            "Mail Addressed to My Full Name",
            "Filter Before My Rules",
        ],
    ),
];

#[tokio::test]
async fn each_page_draws_its_rows_in_order() {
    let built = work();
    for (page, labels) in KEYED {
        let (dom, _seen) = opened_on(&built, *page);
        let markup = dioxus_ssr::render(&dom);
        let mut from = 0;
        for label in *labels {
            let at = markup[from..]
                .find(&format!(">{label}<"))
                .unwrap_or_else(|| panic!("{page:?} has no {label:?} in order: {markup}"));
            from += at;
        }
    }
}

/// Every key of the schema is a row of one page, but the two left on purpose: the provider marks
/// are detent's to set, and the trusted senders and the muted accounts are lists drawn by rows of
/// their own. No key is on
/// two pages.
#[test]
fn every_schema_key_is_on_one_page_or_left_to_detent_on_purpose() {
    let named: Vec<&str> = super::PAGES
        .iter()
        .flat_map(|page| page.sections().iter())
        .flat_map(|section| section.lines.iter())
        .filter_map(|line| match line {
            super::pages::Line::Key(path)
            | super::pages::Line::Folder(path)
            | super::pages::Line::Address(path) => Some(*path),
            super::pages::Line::TrustedSenders
            | super::pages::Line::DefaultMail
            | super::pages::Line::Vips
            | super::pages::Line::RelayKey
            | super::pages::Line::MutedAccounts => None,
        })
        .collect();
    let left = [
        "window.provider_marks",
        "reading.trusted_image_senders",
        "notifications.muted_accounts",
    ];
    for key in mail_ui_model::settings::schema().key {
        let path = key.path.0.as_str();
        let times = named.iter().filter(|named| **named == path).count();
        assert!(
            times == usize::from(!left.contains(&path)),
            "{path} is named on {times} pages"
        );
    }
    for path in named {
        assert!(
            mail_ui_model::settings::schema()
                .key
                .iter()
                .any(|key| key.path.0 == path),
            "a page names {path}, which the schema has not"
        );
    }
}

/// The pages' controls each write `settings.toml`: two switches, on the pages that hold them.
#[tokio::test]
async fn page_controls_write_settings_toml() {
    let built = work();
    let (mut dom, seen) = opened_on(&built, PageId::VIEWING);
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
    let (mut dom, seen) = opened_on(&built, PageId::COMPOSING);
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

/// Each account has a switch in Notifications: off puts its address, in lowercase, on the muted
/// list, and on takes it off again.
#[tokio::test]
async fn an_account_s_switch_in_notifications_mutes_and_unmutes_it() {
    let built = work();
    let client = crate::ui::fixtures::client(&built.store);
    let address = crate::ui::data::account_rows(&client)
        .first()
        .map(|row| row.address.clone())
        .expect("the fixture has an account");
    let (mut dom, seen) = opened_on(&built, PageId::NOTIFICATIONS);
    assert!(stored(&built).notifications.muted_accounts.is_empty());
    // The switch carries its own label (its `aria-checked` is set before it), so it is the one
    // element labelled with the address.
    let switch = seen.one("aria-label", &address);
    click(&mut dom, switch);
    assert_eq!(
        stored(&built).notifications.muted_accounts,
        [address.to_lowercase()],
        "switched off, muted"
    );
    click(&mut dom, switch);
    assert!(
        stored(&built).notifications.muted_accounts.is_empty(),
        "switched on again, heard"
    );
}

/// A login item that records what it is asked, and refuses when told to.
#[derive(Default)]
struct LoginRecorder {
    asked: Mutex<Vec<&'static str>>,
    refuse: bool,
}

impl LoginRecorder {
    fn asked(&self) -> Vec<&'static str> {
        self.asked
            .lock()
            .map(|asked| asked.clone())
            .unwrap_or_default()
    }
}

impl mail_os::login_item::LoginItem for LoginRecorder {
    fn register(&self) -> Result<mail_os::login_item::Registered, String> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push("register");
        }
        if self.refuse {
            Err("Operation not permitted".to_owned())
        } else {
            Ok(mail_os::login_item::Registered::Done)
        }
    }

    fn unregister(&self) -> Result<(), String> {
        if let Ok(mut asked) = self.asked.lock() {
            asked.push("unregister");
        }
        Ok(())
    }
}

/// The Settings window over `built` on General, its login item `item`.
fn general_with_login(built: &Work, item: Arc<LoginRecorder>) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone())
        .with_root_context(mail_os::login_item::LoginItems(item));
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    (dom, seen)
}

/// "Check for mail when quit" is off until it is turned on, and each turn of its switch is
/// handed to the login item: on registers `mailo watch` at login, off unregisters it.
#[tokio::test]
async fn check_for_mail_when_quit_registers_and_unregisters_the_login_item() {
    use mail_ui_model::settings::WhileQuit;
    let built = work();
    let item = Arc::new(LoginRecorder::default());
    let (mut dom, seen) = general_with_login(&built, item.clone());
    assert_eq!(
        stored(&built).sync.while_quit,
        WhileQuit::Off,
        "off as it ships"
    );
    let switch = seen.after("aria-label", "Check for mail when quit", "aria-checked")[0];
    click(&mut dom, switch);
    assert_eq!(stored(&built).sync.while_quit, WhileQuit::On);
    // The login item is asked off the thread that draws, so it is asked once the window settles.
    crate::ui::fixtures::settle_quiet(&mut dom).await;
    assert_eq!(item.asked(), ["register"]);
    click(&mut dom, switch);
    assert_eq!(stored(&built).sync.while_quit, WhileQuit::Off);
    crate::ui::fixtures::settle_quiet(&mut dom).await;
    assert_eq!(item.asked(), ["register", "unregister"]);
}

/// A login item the system would not keep turns the switch off again, and says why.
#[tokio::test]
async fn a_login_item_the_system_refuses_leaves_the_switch_off_and_says_so() {
    use mail_ui_model::settings::WhileQuit;
    let built = work();
    let item = Arc::new(LoginRecorder {
        refuse: true,
        ..LoginRecorder::default()
    });
    let (mut dom, seen) = general_with_login(&built, item.clone());
    let switch = seen.after("aria-label", "Check for mail when quit", "aria-checked")[0];
    click(&mut dom, switch);
    crate::ui::fixtures::settle_quiet(&mut dom).await;
    assert_eq!(item.asked(), ["register"]);
    assert_eq!(stored(&built).sync.while_quit, WhileQuit::Off);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(mail_ui_model::copy::titles::LOGIN_ITEM_NOT_CHANGED)
            && page.contains("Operation not permitted"),
        "{page}"
    );
}

/// A window with no login item (Linux's, where the installed unit checks mail whatever a switch
/// says, and a window the launch did not make) draws no "Check for mail when quit" at all.
#[tokio::test]
async fn without_a_login_item_there_is_no_switch() {
    let built = work();
    let (dom, _) = opened_on(&built, PageId::GENERAL);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Search server"), "General is drawn: {page}");
    assert!(!page.contains("Check for mail when quit"), "{page}");
}

/// Each page, by the sidebar's row, and a line only that page draws.
const PAGES: &[(PageId, &str)] = &[
    (PageId::GENERAL, "Search server"),
    (PageId::ACCOUNTS, mail_ui_model::copy::controls::ADD_ACCOUNT),
    (PageId::CONTACTS, mail_ui_model::copy::controls::NEW_CONTACT),
    (PageId::SIGNATURES, "id=\"signatures-pane\""),
    (PageId::SPACES, mail_ui_model::copy::controls::NEW_SPACE),
    (PageId::COMPOSING, "Check spelling"),
    (PageId::VIEWING, "Brand logos"),
    (PageId::NOTIFICATIONS, "aria-label=\"Notifications\""),
    (PageId::PRIVACY, "Load remote images"),
    (PageId::JUNK_MAIL, "Filter Junk Mail"),
    (PageId::RULES, mail_ui_model::copy::controls::NEW_RULE),
    (PageId::VACATION, "id=\"vacation-pane\""),
    (PageId::KEYBOARD, "Change the key for"),
];

#[test]
fn settings_has_13_pages_in_that_order() {
    assert_eq!(
        super::PAGES
            .iter()
            .map(|page| page.name())
            .collect::<Vec<_>>(),
        [
            mail_ui_model::copy::titles::GENERAL,
            mail_ui_model::copy::titles::ACCOUNTS,
            mail_ui_model::copy::titles::CONTACTS,
            mail_ui_model::copy::titles::SIGNATURES,
            mail_ui_model::copy::titles::SPACES,
            mail_ui_model::copy::titles::COMPOSING,
            mail_ui_model::copy::titles::VIEWING,
            mail_ui_model::copy::titles::NOTIFICATIONS,
            mail_ui_model::copy::titles::PRIVACY,
            mail_ui_model::copy::titles::JUNK_MAIL,
            mail_ui_model::copy::titles::RULES,
            "Vacation Reply",
            mail_ui_model::copy::titles::KEYBOARD,
        ]
    );
}

#[tokio::test]
async fn every_page_is_drawn_in_the_window_from_its_sidebar_row() {
    let built = work();
    assert_eq!(
        PAGES.iter().map(|(page, _)| *page).collect::<Vec<_>>(),
        super::PAGES
            .iter()
            .map(|page| page.id())
            .collect::<Vec<_>>(),
        "a page has no case here"
    );
    for (page, drawn) in PAGES {
        let (dom, _seen) = opened_on(&built, *page);
        let markup = dioxus_ssr::render(&dom);
        assert!(
            markup.contains(&format!("data-page=\"{}\"", page_of(*page).name())),
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
    let (mut dom, seen) = opened_on(&built, PageId::VACATION);
    let list = dioxus_ssr::render(&dom);
    let rows: Vec<_> = crate::ui::data::account_rows(&crate::ui::fixtures::client(&built.store))
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
const ADDS: &[(PageId, &str, &str)] = &[
    (
        PageId::ACCOUNTS,
        mail_ui_model::copy::controls::ADD_ACCOUNT,
        mail_ui_model::copy::aria::ADD_ACCOUNT,
    ),
    (
        PageId::SPACES,
        mail_ui_model::copy::controls::NEW_SPACE,
        mail_ui_model::copy::aria::ADD_A_SPACE,
    ),
    (
        PageId::RULES,
        mail_ui_model::copy::controls::NEW_RULE,
        mail_ui_model::copy::aria::ADD_A_RULE,
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
    asked.ask(SettingsAt::Page(PageId::RULES));
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone())
        .with_root_context(asked.clone());
    let _ = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("data-page=\"Rules\""),
        "it opened elsewhere: {page}"
    );

    asked.ask(SettingsAt::Page(PageId::KEYBOARD));
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
    mail_ui_model::settings::store(mail_ui_model::settings::root_for(&built.dirs.config))
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
            &mail_ui_model::copy::aria::stop_loading_images("news@example.test"),
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
    mail_ui_model::settings::store(mail_ui_model::settings::root_for(&built.dirs.config))
        .save(&settings)
        .unwrap_or_else(|why| panic!("{why:?}"));
    let (dom, _) = trusted_senders(&built);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(mail_ui_model::copy::empty::NO_TRUSTED_SENDERS),
        "{page}"
    );
}

/// Viewing's folder row: Choose… writes the folder the dialog answered as the one Save writes
/// into, and Use Downloads puts the default back. The dialog is the test's: no real one opens.
#[tokio::test]
async fn choose_sets_the_downloads_folder_and_use_downloads_clears_it() {
    use crate::ui::pick::{Ask, Dialogs};
    let built = work();
    let folder = tempfile::tempdir().unwrap_or_else(|why| panic!("a temp dir: {why}"));
    let (dialogs, asked) = Dialogs::answering(vec![folder.path().to_path_buf()]);
    let (mut dom, seen) = opened_on(&built, PageId::VIEWING);
    dom.provide_root_context(dialogs);
    assert_eq!(stored(&built).attachments.downloads_folder, "", "before");

    let mut seen = click(&mut dom, seen.one("id", super::pages::CHOOSE_FOLDER_ID));
    let wanted = folder.path().display().to_string();
    crate::ui::fixtures::settle_until(
        &mut dom,
        &mut seen,
        std::time::Duration::from_millis(150),
        std::time::Duration::from_secs(10),
        |_| stored(&built).attachments.downloads_folder == wanted,
    )
    .await;
    assert_eq!(
        asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(ask, _)| *ask)
            .collect::<Vec<_>>(),
        [Ask::Folder]
    );
    let seen = seen.merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(&wanted),
        "the row does not show the folder: {page}"
    );

    click(&mut dom, seen.one("id", super::pages::USE_DOWNLOADS_ID));
    assert_eq!(
        stored(&built).attachments.downloads_folder,
        "",
        "after Use Downloads"
    );
}

/// Notifications › VIPs: a person pinned in a Space is offered until added, a VIP is a row whose Remove
/// takes the mark off, and the store is what both write.
#[tokio::test]
async fn the_vips_page_offers_the_pinned_people_and_removes_a_vip() {
    use mail_core::Store as _;
    let built = work();
    let (mut dom, seen) = opened_on(&built, PageId::NOTIFICATIONS);
    let seen = click(
        &mut dom,
        seen.one(
            "id",
            &super::pages::vips_row(page_of(PageId::NOTIFICATIONS).name()),
        ),
    )
    .merge(drain_seen(&mut dom));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(mail_ui_model::copy::empty::NO_VIPS), "{page}");
    assert!(
        page.contains(mail_ui_model::copy::headers::PINNED_IN_SPACES),
        "{page}"
    );
    let dana = "dana@example.com";
    let held = |built: &Work| -> Vec<String> {
        built
            .store
            .vips()
            .unwrap()
            .into_iter()
            .map(|vip| vip.address)
            .collect()
    };
    // The store is written on a blocking thread: the test waits for it, then for the page that
    // reads the store again.
    let mut seen = click(
        &mut dom,
        seen.one("aria-label", &mail_ui_model::copy::aria::add_vip(dana)),
    )
    .merge(drain_seen(&mut dom));
    let quiet = std::time::Duration::from_millis(100);
    let bound = std::time::Duration::from_secs(10);
    crate::ui::fixtures::settle_until(&mut dom, &mut seen, quiet, bound, |_| {
        held(&built) == [dana]
    })
    .await;
    let seen = seen.merge(crate::ui::fixtures::settle_quiet_seen(&mut dom).await);
    assert_eq!(held(&built), [dana]);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains(&mail_ui_model::copy::aria::add_vip(dana)),
        "she is still offered: {page}"
    );
    let mut seen = seen;
    click(
        &mut dom,
        seen.one("aria-label", &mail_ui_model::copy::aria::remove_vip(dana)),
    );
    crate::ui::fixtures::settle_until(&mut dom, &mut seen, quiet, bound, |_| {
        held(&built).is_empty()
    })
    .await;
    assert!(held(&built).is_empty());
}

#[tokio::test]
async fn turning_to_another_page_leaves_a_pushed_page_behind() {
    let built = work();
    let asked = super::SettingsAsked::default();
    asked.ask(SettingsAt::Page(PageId::PRIVACY));
    dispatching();
    let mut dom = VirtualDom::new(settings_root)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone())
        .with_root_context(asked.clone());
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    click(&mut dom, seen.one("id", "privacy-pane-open-trusted"));
    asked.ask(SettingsAt::Page(PageId::VIEWING));
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
        page.contains("data-page=\"Viewing\"") && !page.contains("viewing-pane-back-detail"),
        "Viewing opened on a page pushed over Privacy: {page}"
    );
}

/// Privacy draws the rows of the way images are protected and no others: a relay's for a relay, a
/// proxy's for a proxy, none while they go straight.
#[test]
fn privacy_draws_only_the_rows_of_the_way_chosen() {
    use super::pages::{Line, drawn};
    use mail_ui_model::settings::Protect;
    let relay = [
        Line::Key("privacy.relay_kind"),
        Line::Address("privacy.relay"),
        Line::RelayKey,
    ];
    let proxy = [Line::Address("privacy.proxy")];
    for protect in [Protect::Off, Protect::Relay, Protect::Proxy] {
        for line in relay {
            assert_eq!(
                drawn(line, protect),
                protect == Protect::Relay,
                "{line:?} {protect:?}"
            );
        }
        for line in proxy {
            assert_eq!(
                drawn(line, protect),
                protect == Protect::Proxy,
                "{line:?} {protect:?}"
            );
        }
        assert!(drawn(Line::Key("privacy.protect"), protect), "{protect:?}");
    }
}

/// The wait before a conversation is marked read is a row only while marking it read is "After a
/// Delay"; every other row of the page is there under any choice.
#[test]
fn the_mark_read_wait_is_drawn_only_after_a_delay() {
    use super::pages::{Line, shown};
    use mail_ui_model::settings::MarkRead;
    for mark in [MarkRead::OnOpen, MarkRead::AfterDelay, MarkRead::Never] {
        let mut settings = MailSettings::default();
        settings.reading.mark_read = mark;
        assert_eq!(
            shown(Line::Key("reading.mark_read_delay"), &settings),
            mark == MarkRead::AfterDelay,
            "{mark:?}"
        );
        assert!(shown(Line::Key("reading.mark_read"), &settings), "{mark:?}");
        assert!(shown(Line::Key("reading.text_zoom"), &settings), "{mark:?}");
    }
}

/// A relay or proxy that cannot be used is said under its own field, and a page for the way not
/// chosen does not show the other's rows.
#[tokio::test]
async fn an_address_that_cannot_be_used_is_said_under_its_field() {
    use mail_ui_model::settings::{Protect, RouteProblem};
    let built = work();
    let mut settings = MailSettings::default();
    settings.privacy.protect = Protect::Relay;
    settings.privacy.relay = "http://relay.example.test".to_owned();
    mail_ui_model::settings::store(mail_ui_model::settings::root_for(&built.dirs.config))
        .save(&settings)
        .unwrap_or_else(|why| panic!("{why:?}"));
    let (dom, _) = opened_on(&built, PageId::PRIVACY);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(mail_ui_model::copy::fields::route_problem(
            RouteProblem::Relay
        )),
        "{page}"
    );
    // The rows, not the words: the Protect popup names every way (Off, Relay, Proxy) whichever
    // is chosen.
    assert!(
        page.contains("aria-label=\"Relay\" aria-invalid=\"true\""),
        "{page}"
    );
    assert!(!page.contains("aria-label=\"Proxy\""), "{page}");
    assert!(
        !page.contains(mail_ui_model::copy::fields::route_problem(
            RouteProblem::Proxy
        )),
        "{page}"
    );
}

/// The junk switches have a page of their own, listed after Privacy, and General no longer holds
/// them.
#[test]
fn the_junk_switches_are_on_their_own_page_after_privacy() {
    let all: Vec<PageId> = super::PAGES.iter().map(|page| page.id()).collect();
    let privacy = all.iter().position(|page| *page == PageId::PRIVACY);
    let junk = all.iter().position(|page| *page == PageId::JUNK_MAIL);
    assert_eq!(junk, privacy.map(|at| at + 1));
    let keys = |page: PageId| -> Vec<&'static str> {
        page_of(page)
            .sections()
            .iter()
            .flat_map(|section| section.lines.iter())
            .filter_map(|line| match line {
                super::pages::Line::Key(path) => Some(*path),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        keys(PageId::JUNK_MAIL),
        [
            "junk.filter",
            "junk.treat",
            "junk.spare_contacts",
            "junk.spare_written_to",
            "junk.spare_full_name",
            "junk.trust_server",
            "junk.order",
        ]
    );
    assert!(
        keys(PageId::GENERAL)
            .iter()
            .all(|path| !path.starts_with("junk."))
    );
    assert!(
        page_of(PageId::GENERAL)
            .sections()
            .iter()
            .all(|section| section.title != Some("Junk Mail"))
    );
}

#[tokio::test]
async fn the_general_page_no_longer_draws_the_junk_mail_group() {
    let built = work();
    let (dom, _seen) = opened_on(&built, PageId::GENERAL);
    let markup = dioxus_ssr::render(&dom);
    assert!(!markup.contains("Filter Junk Mail"), "{markup}");
    assert!(!markup.contains("Never Junk Mail From"), "{markup}");
}

/// Every page's id is its own, and each of the ids the shell and the deep links name is a page
/// of the sidebar.
#[test]
fn every_page_id_names_one_page_of_the_sidebar() {
    let named = [
        PageId::GENERAL,
        PageId::ACCOUNTS,
        PageId::CONTACTS,
        PageId::SIGNATURES,
        PageId::SPACES,
        PageId::COMPOSING,
        PageId::VIEWING,
        PageId::NOTIFICATIONS,
        PageId::PRIVACY,
        PageId::JUNK_MAIL,
        PageId::RULES,
        PageId::VACATION,
        PageId::KEYBOARD,
    ];
    let listed: Vec<PageId> = super::PAGES.iter().map(|page| page.id()).collect();
    assert_eq!(listed, named, "the sidebar lists the ids in this order");
    for (at, id) in named.iter().enumerate() {
        assert!(!named[..at].contains(id), "{id:?} names two pages");
    }
}
