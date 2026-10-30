//! The pages that draw mailo's settings: General, Composing, Viewing, Notifications, Privacy and Junk Mail.
//!
//! Each page is a list of groups of [`Line`]s ([`super::page::Page::sections`]): a key of the
//! schema, drawn by its kind ([`super::keys`]) as detent draws it, the row that opens the trusted senders, or the row
//! that sets the default mail application. A key is on a page once it is named there, and the
//! tests hold every key of the schema to being named or left to detent on purpose (the provider
//! marks). The help the schema gives each key is detent's; the pages draw no captions.
//!
//! The trusted senders are a page of Privacy's own, as System Settings pushes a list it keeps: a
//! row per sender with its Remove. The pages are quire's `PaneStack`, its path each page's own.

use super::keys::{KeyRow, value_of};
use super::rows::link_row;
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::fields::text_field_model::Invalid;
use ds::components::lists::list::model::ListStyle;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use ds_settings::schema::{Help, KeySpec};

/// One row of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Line {
    /// The schema's key at this path.
    Key(&'static str),
    /// The row that opens the trusted senders' page.
    TrustedSenders,
    /// The row that opens the VIPs' page ([`super::vips`]).
    Vips,
    /// Which application opens email links, with Make Default (`crate::ui::default_mail`).
    DefaultMail,
    /// A switch per account: whether its new mail is announced (`notifications.muted_accounts`).
    MutedAccounts,
    /// The privacy relay's key: a field that keeps what is typed in the keyring, never in the
    /// settings file ([`RelayKey`]).
    RelayKey,
    /// The schema's text key at this path, holding an address: a field that says what is wrong
    /// with it while the way it names cannot be used ([`AddressRow`]).
    Address(&'static str),
    /// The schema's text key at this path, holding a folder: drawn as the folder, with Choose…
    /// to pick another in the system's dialog, and Use Downloads to go back to the default.
    Folder(&'static str),
}

/// A group of a page: its header, when it has one, and its rows in the order drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct Section {
    pub title: Option<&'static str>,
    pub lines: &'static [Line],
}

/// A page of a settings page's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageAt {
    Root,
    TrustedSenders,
    Vips,
}

/// The stack's element id: its back button is `<id>-back-detail`, as `general-pane-back-detail`.
fn stack(name: &str) -> String {
    format!("{}-pane", name.to_lowercase())
}

/// The element id of the row that opens the trusted senders, where the keyboard goes back to.
fn trusted_row(name: &str) -> String {
    format!("{}-open-trusted", stack(name))
}

/// The element id of the row that opens the VIPs, where the keyboard goes back to.
pub(super) fn vips_row(name: &str) -> String {
    format!("{}-open-vips", stack(name))
}

/// One of the pages that draw the schema: its groups at the root, the trusted senders (on Privacy)
/// or the VIPs (on Notifications) pushed over them.
#[component]
pub(super) fn Page(name: &'static str, sections: &'static [Section]) -> Element {
    let mut path = use_signal(|| PanePath::new(PageAt::Root));
    let title = use_callback(move |at: PageAt| {
        match at {
            PageAt::Root => name,
            PageAt::TrustedSenders => copy::titles::TRUSTED_SENDERS,
            PageAt::Vips => copy::titles::VIPS,
        }
        .to_owned()
    });
    let shown = use_callback(move |at: PageAt| match at {
        PageAt::Root => rsx! {
            Groups {
                name,
                sections,
                on_trusted: move |()| {
                    let next = path.peek().pushed(PageAt::TrustedSenders);
                    path.set(next);
                },
                on_vips: move |()| {
                    let next = path.peek().pushed(PageAt::Vips);
                    path.set(next);
                },
            }
        },
        PageAt::TrustedSenders => rsx! { TrustedSenders {} },
        PageAt::Vips => rsx! { super::vips::VipsPage {} },
    });
    let opener = use_callback(move |at: PageAt| match at {
        PageAt::Root => None,
        PageAt::TrustedSenders => Some(trusted_row(name)),
        PageAt::Vips => Some(vips_row(name)),
    });
    rsx! {
        PaneStack::<PageAt> {
            path: path(),
            title,
            page: shown,
            opener: Some(opener),
            on_back: move |()| {
                let back = path.peek().popped();
                path.set(back);
            },
            common: Common { id: Some(stack(name)), ..Common::default() },
        }
    }
}

/// The schema's key at `path`, its help left to detent.
fn key_at(path: &str) -> Option<KeySpec> {
    crate::settings::schema()
        .key
        .into_iter()
        .find(|key| key.path.0 == path)
        .map(|key| KeySpec {
            help: Help(String::new()),
            ..key
        })
}

/// Whether `line` is drawn while images are protected as `protect` says: the relay's rows only
/// for a relay, the proxy's only for a proxy, since a row for the way not chosen sets nothing.
pub(super) fn drawn(line: Line, protect: crate::settings::Protect) -> bool {
    use crate::settings::Protect;
    match line {
        Line::Key("privacy.relay_kind") | Line::Address("privacy.relay") | Line::RelayKey => {
            protect == Protect::Relay
        }
        Line::Address("privacy.proxy") => protect == Protect::Proxy,
        _ => true,
    }
}

/// Whether `line` is drawn under `settings`: the rows of the way images are protected
/// ([`drawn`]), and the wait before a conversation is marked read only while marking it read is
/// "After a Delay", since a wait under any other choice sets nothing.
pub(super) fn shown(line: Line, settings: &crate::settings::MailSettings) -> bool {
    match line {
        Line::Key("reading.mark_read_delay") => {
            settings.reading.mark_read == crate::settings::MarkRead::AfterDelay
        }
        _ => drawn(line, settings.privacy.protect),
    }
}

/// The root: each group's rows.
#[component]
fn Groups(
    name: &'static str,
    sections: &'static [Section],
    on_trusted: EventHandler<()>,
    on_vips: EventHandler<()>,
) -> Element {
    let settings = crate::ui::prefs::current();
    let values =
        toml::Value::try_from(&settings).unwrap_or_else(|_| toml::Value::Table(toml::Table::new()));
    let mut failed = use_signal(|| None::<String>);
    let mut login = use_signal(|| None::<(&'static str, String)>);
    // "Check for mail when quit" is drawn only where the window has a login item to keep it:
    // the launched app's on macOS and Windows (`login_item::platform`). Elsewhere the switch
    // would say what nothing does.
    let keeps = try_consume_context::<crate::login_item::LoginItems>().is_some();
    let onedit = move |(path, value): (String, toml::Value)| {
        let changed = crate::ui::prefs::change_key(&path, value);
        if changed.is_ok() && path == crate::settings::WHILE_QUIT {
            follow_login_item(login);
        }
        failed.set(changed.err());
    };
    rsx! {
        Form {
            for (at, section) in sections.iter().enumerate() {
                FormSection { key: "{at}", title: section.title.map(str::to_owned),
                    for line in section.lines.iter().copied().filter(|line| shown(*line, &settings)) {
                        match line {
                            Line::Key(path) if path == crate::settings::WHILE_QUIT && !keeps => rsx! {},
                            Line::Key(path) => match key_at(path) {
                                Some(spec) => rsx! {
                                    KeyRow { key: "{path}", value: value_of(&values, &spec), spec, onedit }
                                },
                                None => rsx! {},
                            },
                            Line::DefaultMail => rsx! { crate::ui::default_mail::DefaultMailRow {} },
                            Line::MutedAccounts => rsx! { MutedAccounts { failed } },
                            Line::RelayKey => rsx! { RelayKey { failed } },
                            Line::Address(path) => match key_at(path) {
                                Some(spec) => rsx! {
                                    AddressRow { key: "{path}", value: value_of(&values, &spec), spec, onedit }
                                },
                                None => rsx! {},
                            },
                            Line::Folder(path) => match key_at(path) {
                                Some(spec) => rsx! {
                                    FolderRow { key: "{path}", value: value_of(&values, &spec), spec, onedit }
                                },
                                None => rsx! {},
                            },
                            Line::Vips => rsx! {
                                List::<String> {
                                    label: copy::titles::VIPS,
                                    style: ListStyle::Grouped,
                                    items: vec![link_row(
                                        "vips",
                                        copy::titles::VIPS,
                                        RowLeading::None,
                                        move |_| on_vips.call(()),
                                        Common { id: Some(vips_row(name)), ..Common::default() },
                                    )],
                                }
                            },
                            Line::TrustedSenders => rsx! {
                                List::<String> {
                                    label: copy::titles::TRUSTED_SENDERS,
                                    style: ListStyle::Grouped,
                                    items: vec![link_row(
                                        "trusted",
                                        copy::titles::TRUSTED_SENDERS,
                                        RowLeading::None,
                                        move |_| on_trusted.call(()),
                                        Common { id: Some(trusted_row(name)), ..Common::default() },
                                    )],
                                }
                            },
                        }
                    }
                }
            }
        }
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| failed.set(None) }
        }
        if let Some((title, why)) = login() {
            Problem { title, why, onclose: move |()| login.set(None) }
        }
    }
}

/// Hand "Check for mail when quit", as it now stands, to the login item (`crate::login_item`)
/// off the thread that draws, since `launchctl`, SMAppService and `reg` take their time, and put
/// in `login` what came of it when there is something to say. A window with no login item
/// changes the setting alone.
fn follow_login_item(mut login: Signal<Option<(&'static str, String)>>) {
    use crate::login_item::{LoginItems, apply};
    let Some(items) = try_consume_context::<LoginItems>() else {
        return;
    };
    let wanted = crate::ui::prefs::current().sync.while_quit;
    spawn(async move {
        let applied = tokio::task::spawn_blocking(move || apply(&*items.0, wanted))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        login.set(said_of(applied, wanted));
    });
}

/// What to say of the login item asked for `wanted`, which answered `applied`: a title and its
/// message, or nothing.
///
/// A switch turned on that the system would not keep is turned off again, so it never says mail
/// is checked when it is not. One turned off that the system would not let go of stays off: the
/// person said off, and the message says the login item may still run.
fn said_of(
    applied: Result<Option<crate::login_item::Registered>, String>,
    wanted: crate::settings::WhileQuit,
) -> Option<(&'static str, String)> {
    use crate::login_item::Registered;
    use crate::settings::WhileQuit;
    match (applied, wanted) {
        (Ok(Some(Registered::NeedsApproval)), _) => Some((
            copy::titles::ALLOW_LOGIN_ITEM,
            copy::errors::ALLOW_LOGIN_ITEM.to_owned(),
        )),
        (Ok(_), _) => None,
        (Err(why), WhileQuit::On) => {
            let mut said = copy::errors::login_item_not_registered(&why);
            if let Err(also) =
                crate::ui::prefs::change(|settings| settings.sync.while_quit = WhileQuit::Off)
            {
                said = format!("{said} {also}");
            }
            Some((copy::titles::LOGIN_ITEM_NOT_CHANGED, said))
        }
        (Err(why), WhileQuit::Off) => Some((
            copy::titles::LOGIN_ITEM_NOT_CHANGED,
            copy::errors::login_item_not_removed(&why),
        )),
    }
}

/// A text key that holds an address: a field, written when it is left, that says what is wrong
/// with it while the way it names cannot be used ([`crate::ui::prefs::route_problem`]). Images
/// wait on it, and the field is where the person is told.
#[component]
fn AddressRow(
    spec: KeySpec,
    value: toml::Value,
    onedit: EventHandler<(String, toml::Value)>,
) -> Element {
    let path = spec.path.0.clone();
    let mut typed = use_signal(|| value.as_str().unwrap_or_default().to_owned());
    let ours = match path.as_str() {
        "privacy.relay" => Some(crate::settings::RouteProblem::Relay),
        "privacy.proxy" => Some(crate::settings::RouteProblem::Proxy),
        _ => None,
    };
    // An address is wrong whatever the relay's key is, so it is judged from the settings alone: the
    // window's, which this row follows as they change, with or without the keyring having answered.
    let problem = crate::ui::prefs::current().privacy.route(None).err();
    let validity = match problem.filter(|problem| Some(*problem) == ours) {
        None => Validity::Valid,
        Some(problem) => {
            let why = copy::fields::route_problem(problem);
            Validity::Invalid(Invalid {
                message: why.to_owned().into(),
                stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
            })
        }
    };
    rsx! {
        FieldRow { label: TextLine::from(spec.label.0.clone()),
            TextField {
                label: spec.label.0.clone(),
                value: typed(),
                validity,
                oninput: move |text: String| typed.set(text),
                onblur: move |()| {
                    let text = typed.peek().trim().to_owned();
                    onedit.call((path.clone(), toml::Value::String(text)));
                },
            }
        }
    }
}

/// A folder key's row: the folder Save writes into now (the default's when the key is empty),
/// Choose… to pick another, and Use Downloads while one is chosen.
#[component]
fn FolderRow(
    spec: KeySpec,
    value: toml::Value,
    onedit: EventHandler<(String, toml::Value)>,
) -> Element {
    // A value that is not text is no folder: the default's.
    let chosen = value.as_str().unwrap_or_default().trim().to_owned();
    let shown = crate::ui::files::save_dir();
    let path = spec.path.0.clone();
    let reset = path.clone();
    rsx! {
        FieldRow { label: TextLine::from(spec.label.0.clone()),
            Label { text: shown.display().to_string(), common: crate::ui::common::classed("truncate") }
            Button {
                label: copy::controls::CHOOSE_FOLDER,
                size: ControlSize::Small,
                common: Common {
                    aria_label: Some(copy::aria::CHOOSE_DOWNLOADS_FOLDER.to_owned()),
                    id: Some(CHOOSE_FOLDER_ID.to_owned()),
                    ..Common::default()
                },
                onclick: on_primary(move || {
                    let path = path.clone();
                    crate::ui::pick::choose(crate::ui::pick::Ask::Folder, Some(shown.clone()), move |folders| {
                        if let Some(folder) = folders.into_iter().next() {
                            onedit.call((path, toml::Value::String(folder.display().to_string())));
                        }
                    });
                }),
            }
            if !chosen.is_empty() {
                Button {
                    label: copy::controls::USE_DOWNLOADS,
                    size: ControlSize::Small,
                    common: Common { id: Some(USE_DOWNLOADS_ID.to_owned()), ..Common::default() },
                    onclick: on_primary(move || onedit.call((reset.clone(), toml::Value::String(String::new())))),
                }
            }
        }
    }
}

/// The Choose… of the folder Save writes into.
pub const CHOOSE_FOLDER_ID: &str = "general-choose-downloads";

/// The Use Downloads of the folder Save writes into.
pub const USE_DOWNLOADS_ID: &str = "general-use-downloads";

/// A switch for each account: on when its new mail is announced. Switching one off adds its
/// address to `notifications.muted_accounts`, in lowercase, and back on takes it out.
#[component]
fn MutedAccounts(mut failed: Signal<Option<String>>) -> Element {
    let rows = crate::ui::data::use_account_rows()();
    let muted = crate::ui::prefs::current().notifications.muted_accounts;
    rsx! {
        if rows.is_empty() {
            FieldRow { label: copy::empty::NO_ACCOUNTS_TO_NOTIFY }
        }
        for row in rows {
            {
                let address = row.address.to_lowercase();
                let on = !muted.contains(&address);
                rsx! {
                    Toggle {
                        key: "{address}",
                        label: row.address.clone(),
                        value: if on { Check::On } else { Check::Off },
                        onchange: move |next| {
                            let address = address.clone();
                            let changed = crate::ui::prefs::change(move |settings| {
                                let list = &mut settings.notifications.muted_accounts;
                                list.retain(|kept| *kept != address);
                                if next == Check::Off {
                                    list.push(address);
                                }
                            });
                            failed.set(changed.err());
                        },
                    }
                }
            }
        }
    }
}

/// The privacy relay's key. It is typed here and goes to the keyring, which holds it from then
/// on; the settings file is told only that it changed, so the windows that fetch images read it
/// again. The field never shows a kept key, only that one is kept.
#[component]
fn RelayKey(mut failed: Signal<Option<String>>) -> Element {
    let mut typed = use_signal(String::new);
    let mut kept = use_resource(|| async {
        let Some(client) = crate::ui::live::try_client() else {
            return false;
        };
        tokio::task::spawn_blocking(move || client.net().relay_key())
            .await
            .is_ok_and(|read| read.ok().flatten().is_some())
    });
    let is_kept = (*kept.read()).unwrap_or(false);
    let keep = move |key: String| {
        let Some(client) = crate::ui::live::try_client() else {
            return;
        };
        spawn(async move {
            let done = tokio::task::spawn_blocking(move || client.net().set_relay_key(&key))
                .await
                .unwrap_or_else(|why| Err(why.to_string()));
            match done {
                Ok(()) => {
                    typed.set(String::new());
                    let bumped = crate::ui::prefs::change(|settings| {
                        settings.privacy.relay_key_changed =
                            settings.privacy.relay_key_changed.wrapping_add(1);
                    });
                    failed.set(bumped.err());
                    kept.restart();
                }
                Err(why) => failed.set(Some(crate::ui::remedy::plainly(
                    copy::errors::RELAY_KEY_NOT_KEPT,
                    why,
                ))),
            }
        });
    };
    rsx! {
        FieldRow {
            label: copy::fields::RELAY_KEY,
            help: Some(TextLine::from(if is_kept {
                copy::fields::RELAY_KEY_KEPT
            } else {
                copy::fields::RELAY_KEY_HELP
            })),
            TextField {
                label: copy::fields::RELAY_KEY,
                kind: FieldKind::Secure,
                value: typed(),
                placeholder: if is_kept { copy::fields::RELAY_KEY_PLACEHOLDER_KEPT } else { copy::fields::RELAY_KEY_PLACEHOLDER },
                oninput: move |text: String| typed.set(text),
                onblur: move |()| {
                    let key = typed.peek().trim().to_owned();
                    if !key.is_empty() {
                        keep(key);
                    }
                },
            }
            if is_kept {
                Button {
                    label: copy::controls::FORGET_KEY,
                    onclick: on_primary(move || keep(String::new())),
                }
            }
        }
    }
}

/// The senders whose images load, a row each with its Remove.
#[component]
fn TrustedSenders() -> Element {
    let settings = crate::ui::prefs::current();
    let senders = settings.reading.trusted_image_senders.clone();
    let mut failed = use_signal(|| None::<String>);
    let none = senders.is_empty();
    rsx! {
        Form {
            FormSection {
                if none {
                    FieldRow { label: copy::empty::NO_TRUSTED_SENDERS }
                }
                for sender in senders {
                    FieldRow {
                        key: "{sender}",
                        label: sender.clone(),
                        Button {
                            label: copy::controls::REMOVE,
                            common: Common {
                                aria_label: Some(copy::aria::stop_loading_images(&sender)),
                                ..Common::default()
                            },
                            onclick: {
                                let sender = sender.clone();
                                on_primary(move || {
                                    let gone = sender.clone();
                                    let changed = crate::ui::prefs::change(move |settings| {
                                        settings.reading.trusted_image_senders.retain(|kept| *kept != gone);
                                    });
                                    failed.set(changed.err());
                                })
                            },
                        }
                    }
                }
            }
        }
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| failed.set(None) }
        }
    }
}
