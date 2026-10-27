//! General: six groups of mailo's settings, Reading, Message list, Mail, Composing, Mailboxes
//! and Junk Mail, and the trusted senders' and the VIPs' pages pushed over them.
//!
//! Each group is a list of [`Line`]s ([`SECTIONS`]): a key of the schema, drawn by its kind
//! ([`super::keys`]) as detent draws it, or the row that opens the trusted senders. A key is on
//! the page once it is named there, and the tests hold every key of the schema to being named or
//! left to detent on purpose (the provider marks). The help the schema gives each key is detent's;
//! the page draws no captions.
//!
//! The trusted senders are a page of their own, as System Settings pushes a list it keeps: a row
//! per sender with its Remove. The pages are quire's `PaneStack`, its path this page's own.

use super::keys::{KeyRow, value_of};
use super::rows::link_row;
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use ds_settings::schema::{Help, KeySpec};

/// One row of General.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Line {
    /// The schema's key at this path.
    Key(&'static str),
    /// The row that opens the trusted senders' page.
    TrustedSenders,
    /// The row that opens the VIPs' page ([`super::vips`]).
    Vips,
    /// Which application opens email links, with Make Default (`crate::ui::default_mail`).
    DefaultMail,
    /// The schema's text key at this path, holding a folder: drawn as the folder, with Choose…
    /// to pick another in the system's dialog, and Use Downloads to go back to the default.
    Folder(&'static str),
}

/// A group of General: its header, and its rows in the order drawn.
#[derive(PartialEq)]
pub(super) struct Section {
    pub title: &'static str,
    pub lines: &'static [Line],
}

/// General, group by group.
pub(super) const SECTIONS: [Section; 6] = [
    Section {
        title: copy::headers::READING,
        lines: &[
            Line::Key("reading.remote_images"),
            Line::TrustedSenders,
            Line::Key("reading.brand_logos"),
            Line::Key("reading.prefer_plain_text"),
            Line::Key("reading.mark_read"),
            Line::Key("reading.mark_read_delay"),
            Line::Key("reading.text_zoom"),
            Line::Folder("attachments.downloads_folder"),
            Line::Key("attachments.remove_unedited"),
        ],
    },
    Section {
        title: copy::headers::MESSAGE_LIST,
        lines: &[
            Line::Key("window.snippets"),
            Line::Key("window.label_chips"),
            Line::Key("window.group_by_date"),
        ],
    },
    Section {
        title: copy::headers::MAIL,
        lines: &[
            Line::DefaultMail,
            Line::Key("notifications.new_mail"),
            Line::Key(crate::settings::WHILE_QUIT),
            Line::Key("notifications.notify_for"),
            Line::Vips,
            Line::Key("reading.read_receipts"),
            Line::Key("search.server_automatically"),
        ],
    },
    Section {
        title: copy::headers::COMPOSING,
        lines: &[
            Line::Key("compose.format"),
            Line::Key("compose.reply_same_format"),
            Line::Key("compose.undo_send"),
            Line::Key("compose.spelling"),
            Line::Key("compose.copy_myself"),
            Line::Key("compose.signature_above_quote"),
        ],
    },
    Section {
        title: copy::headers::MAILBOXES,
        lines: &[
            Line::Key("mailboxes.delete"),
            Line::Key("mailboxes.erase_trash"),
            Line::Key("mailboxes.erase_spam"),
        ],
    },
    Section {
        title: copy::headers::JUNK_MAIL,
        lines: &[
            Line::Key("junk.filter"),
            Line::Key("junk.treat"),
            Line::Key("junk.spare_contacts"),
            Line::Key("junk.spare_written_to"),
            Line::Key("junk.spare_full_name"),
            Line::Key("junk.trust_server"),
            Line::Key("junk.order"),
        ],
    },
];

/// A page of General's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneralAt {
    Root,
    TrustedSenders,
    Vips,
}

/// The stack's element id: its back button is `general-pane-back-detail`.
const STACK: &str = "general-pane";

/// The element id of the row that opens the trusted senders, where the keyboard goes back to.
const TRUSTED_ROW: &str = "general-pane-open-trusted";

/// The element id of the row that opens the VIPs.
pub(super) const VIPS_ROW: &str = "general-pane-open-vips";

#[component]
pub(super) fn General(revision: Signal<u64>) -> Element {
    let mut path = use_signal(|| PanePath::new(GeneralAt::Root));
    let title = use_callback(|page: GeneralAt| {
        match page {
            GeneralAt::Root => copy::titles::GENERAL,
            GeneralAt::TrustedSenders => copy::titles::TRUSTED_SENDERS,
            GeneralAt::Vips => copy::titles::VIPS,
        }
        .to_owned()
    });
    let page = use_callback(move |page: GeneralAt| match page {
        GeneralAt::Root => rsx! {
            Groups {
                sections: &SECTIONS,
                on_trusted: move |()| {
                    let next = path.peek().pushed(GeneralAt::TrustedSenders);
                    path.set(next);
                },
                on_vips: move |()| {
                    let next = path.peek().pushed(GeneralAt::Vips);
                    path.set(next);
                },
            }
        },
        GeneralAt::TrustedSenders => rsx! { TrustedSenders {} },
        GeneralAt::Vips => rsx! { super::vips::VipsPage { revision } },
    });
    let opener = use_callback(|page: GeneralAt| match page {
        GeneralAt::Root => None,
        GeneralAt::TrustedSenders => Some(TRUSTED_ROW.to_owned()),
        GeneralAt::Vips => Some(VIPS_ROW.to_owned()),
    });
    rsx! {
        PaneStack::<GeneralAt> {
            path: path(),
            title,
            page,
            opener: Some(opener),
            on_back: move |()| {
                let back = path.peek().popped();
                path.set(back);
            },
            common: Common { id: Some(STACK.to_owned()), ..Common::default() },
        }
    }
}

/// Privacy, a page of its own: what a message's remote content may learn about the reader.
pub(super) const PRIVACY: [Section; 1] = [Section {
    title: copy::titles::PRIVACY,
    lines: &[
        Line::Key("privacy.block_remote_content"),
        Line::Key("privacy.protect"),
        Line::Key("privacy.relay_kind"),
        Line::Key("privacy.relay"),
        Line::Key("privacy.relay_key"),
        Line::Key("privacy.proxy"),
    ],
}];

/// The Privacy page: General's rows, drawn from [`PRIVACY`].
#[component]
pub(super) fn Privacy() -> Element {
    rsx! { Groups { sections: &PRIVACY, on_trusted: move |()| {}, on_vips: move |()| {} } }
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

/// The root: each group's rows.
#[component]
fn Groups(
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
            for section in sections.iter() {
                FormSection { key: "{section.title}", title: Some(section.title.to_owned()),
                    for line in section.lines.iter().copied() {
                        match line {
                            Line::Key(path) if path == crate::settings::WHILE_QUIT && !keeps => rsx! {},
                            Line::Key(path) => match key_at(path) {
                                Some(spec) => rsx! {
                                    KeyRow { key: "{path}", value: value_of(&values, &spec), spec, onedit }
                                },
                                None => rsx! {},
                            },
                            Line::DefaultMail => rsx! { crate::ui::default_mail::DefaultMailRow {} },
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
                                        Common { id: Some(VIPS_ROW.to_owned()), ..Common::default() },
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
                                        Common { id: Some(TRUSTED_ROW.to_owned()), ..Common::default() },
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
