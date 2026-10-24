//! General: two groups of mailo's settings, Reading and Mail, and the trusted senders' page
//! pushed over them.
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
use ds_settings::schema::{Help, KeySpec};

/// One row of General.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Line {
    /// The schema's key at this path.
    Key(&'static str),
    /// The row that opens the trusted senders' page.
    TrustedSenders,
    /// Which application opens email links, with Make Default (`crate::ui::default_mail`).
    DefaultMail,
}

/// A group of General: its header, and its rows in the order drawn.
pub(super) struct Section {
    pub title: &'static str,
    pub lines: &'static [Line],
}

/// General, group by group.
pub(super) const SECTIONS: [Section; 3] = [
    Section {
        title: copy::headers::READING,
        lines: &[
            Line::Key("reading.remote_images"),
            Line::TrustedSenders,
            Line::Key("reading.brand_logos"),
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
            Line::Key("reading.read_receipts"),
            Line::Key("compose.spelling"),
            Line::Key("compose.copy_myself"),
            Line::Key("search.server_automatically"),
        ],
    },
];

/// A page of General's pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneralAt {
    Root,
    TrustedSenders,
}

/// The stack's element id: its back button is `general-pane-back-detail`.
const STACK: &str = "general-pane";

/// The element id of the row that opens the trusted senders, where the keyboard goes back to.
const TRUSTED_ROW: &str = "general-pane-open-trusted";

#[component]
pub(super) fn General() -> Element {
    let mut path = use_signal(|| PanePath::new(GeneralAt::Root));
    let title = use_callback(|page: GeneralAt| {
        match page {
            GeneralAt::Root => copy::titles::GENERAL,
            GeneralAt::TrustedSenders => copy::titles::TRUSTED_SENDERS,
        }
        .to_owned()
    });
    let page = use_callback(move |page: GeneralAt| match page {
        GeneralAt::Root => rsx! {
            Groups {
                on_trusted: move |()| {
                    let next = path.peek().pushed(GeneralAt::TrustedSenders);
                    path.set(next);
                },
            }
        },
        GeneralAt::TrustedSenders => rsx! { TrustedSenders {} },
    });
    let opener = use_callback(|page: GeneralAt| match page {
        GeneralAt::Root => None,
        GeneralAt::TrustedSenders => Some(TRUSTED_ROW.to_owned()),
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
fn Groups(on_trusted: EventHandler<()>) -> Element {
    let settings = crate::ui::prefs::current();
    let values =
        toml::Value::try_from(&settings).unwrap_or_else(|_| toml::Value::Table(toml::Table::new()));
    let mut failed = use_signal(|| None::<String>);
    let mut login = use_signal(|| None::<(&'static str, String)>);
    let onedit = move |(path, value): (String, toml::Value)| {
        let changed = crate::ui::prefs::change_key(&path, value);
        if changed.is_ok() && path == crate::settings::WHILE_QUIT {
            login.set(follow_login_item());
        }
        failed.set(changed.err());
    };
    rsx! {
        Form {
            for section in SECTIONS.iter() {
                FormSection { key: "{section.title}", title: Some(section.title.to_owned()),
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

/// Hand "Check for mail when quit", as it now stands, to the login item (`crate::login_item`),
/// and say what came of it when there is something to say: a title and its message. A window
/// with no login item (a test's, unless it hands a recorder) changes the setting alone.
///
/// A switch turned on that the system would not keep is turned off again, so it never says mail
/// is checked when it is not. One turned off that the system would not let go of stays off: the
/// person said off, and the message says the login item may still run.
fn follow_login_item() -> Option<(&'static str, String)> {
    use crate::login_item::{LoginItems, Registered, apply};
    use crate::settings::WhileQuit;
    let items = try_consume_context::<LoginItems>()?;
    let wanted = crate::ui::prefs::current().sync.while_quit;
    match (apply(&*items.0, wanted), wanted) {
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
