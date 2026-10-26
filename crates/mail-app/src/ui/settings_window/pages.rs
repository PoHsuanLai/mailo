//! The pages that draw mailo's settings: General, Composing, Viewing, Notifications and Privacy.
//!
//! Each page is a list of groups of [`Line`]s ([`sections`]): a key of the schema, drawn by its
//! kind ([`super::keys`]) as detent draws it, the row that opens the trusted senders, or the row
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
use crate::ui::view::SettingsPage;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::prelude::*;
use ds::root::common::Common;
use ds_settings::schema::{Help, KeySpec};

/// One row of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Line {
    /// The schema's key at this path.
    Key(&'static str),
    /// The row that opens the trusted senders' page.
    TrustedSenders,
    /// Which application opens email links, with Make Default (`crate::ui::default_mail`).
    DefaultMail,
}

/// A group of a page: its header, when it has one, and its rows in the order drawn.
pub(super) struct Section {
    pub title: Option<&'static str>,
    pub lines: &'static [Line],
}

/// The groups `page` draws. Accounts, Spaces, Rules, Vacation Reply and Keyboard draw their own.
pub(super) fn sections(page: SettingsPage) -> &'static [Section] {
    match page {
        SettingsPage::General => &[Section {
            title: Some(copy::headers::MAIL),
            lines: &[Line::DefaultMail, Line::Key("search.server_automatically")],
        }],
        SettingsPage::Composing => &[Section {
            title: None,
            lines: &[Line::Key("compose.spelling")],
        }],
        SettingsPage::Viewing => &[Section {
            title: None,
            lines: &[Line::Key("reading.brand_logos")],
        }],
        SettingsPage::Notifications => &[Section {
            title: None,
            lines: &[Line::Key("notifications.new_mail")],
        }],
        SettingsPage::Privacy => &[Section {
            title: None,
            lines: &[
                Line::Key("reading.remote_images"),
                Line::TrustedSenders,
                Line::Key("reading.read_receipts"),
            ],
        }],
        SettingsPage::Accounts
        | SettingsPage::Spaces
        | SettingsPage::Rules
        | SettingsPage::Vacation
        | SettingsPage::Keyboard => &[],
    }
}

/// A page of a settings page's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageAt {
    Root,
    TrustedSenders,
}

/// The stack's element id: its back button is `<id>-back-detail`, as `general-pane-back-detail`.
fn stack(page: SettingsPage) -> String {
    format!("{}-pane", page.name().to_lowercase())
}

/// The element id of the row that opens the trusted senders, where the keyboard goes back to.
fn trusted_row(page: SettingsPage) -> String {
    format!("{}-open-trusted", stack(page))
}

#[component]
pub(super) fn Page(page: SettingsPage) -> Element {
    let mut path = use_signal(|| PanePath::new(PageAt::Root));
    let title = use_callback(move |at: PageAt| {
        match at {
            PageAt::Root => page.name(),
            PageAt::TrustedSenders => copy::titles::TRUSTED_SENDERS,
        }
        .to_owned()
    });
    let shown = use_callback(move |at: PageAt| match at {
        PageAt::Root => rsx! {
            Groups {
                page,
                on_trusted: move |()| {
                    let next = path.peek().pushed(PageAt::TrustedSenders);
                    path.set(next);
                },
            }
        },
        PageAt::TrustedSenders => rsx! { TrustedSenders {} },
    });
    let opener = use_callback(move |at: PageAt| match at {
        PageAt::Root => None,
        PageAt::TrustedSenders => Some(trusted_row(page)),
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
            common: Common { id: Some(stack(page)), ..Common::default() },
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

/// The root: each group's rows.
#[component]
fn Groups(page: SettingsPage, on_trusted: EventHandler<()>) -> Element {
    let settings = crate::ui::prefs::current();
    let values =
        toml::Value::try_from(&settings).unwrap_or_else(|_| toml::Value::Table(toml::Table::new()));
    let mut failed = use_signal(|| None::<String>);
    let onedit = move |(path, value): (String, toml::Value)| {
        failed.set(crate::ui::prefs::change_key(&path, value).err());
    };
    rsx! {
        Form {
            for (at, section) in sections(page).iter().enumerate() {
                FormSection { key: "{at}", title: section.title.map(str::to_owned),
                    for line in section.lines.iter().copied() {
                        match line {
                            Line::Key(path) => match key_at(path) {
                                Some(spec) => rsx! {
                                    KeyRow { key: "{path}", value: value_of(&values, &spec), spec, onedit }
                                },
                                None => rsx! {},
                            },
                            Line::DefaultMail => rsx! { crate::ui::default_mail::DefaultMailRow {} },
                            Line::TrustedSenders => rsx! {
                                List::<String> {
                                    label: copy::titles::TRUSTED_SENDERS,
                                    style: ListStyle::Grouped,
                                    items: vec![link_row(
                                        "trusted",
                                        copy::titles::TRUSTED_SENDERS,
                                        RowLeading::None,
                                        move |_| on_trusted.call(()),
                                        Common { id: Some(trusted_row(page)), ..Common::default() },
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
