//! General: two groups of mailo's settings, Reading and Mail, and the trusted senders' page
//! pushed over them.
//!
//! Each group is a list of [`Line`]s ([`SECTIONS`]): a key of the schema, drawn by its kind
//! ([`super::keys`]) as detent draws it, or the row that opens the trusted senders. A key is on
//! the page once it is named there, and the tests hold every key of the schema to being named or
//! left to detent on purpose (the provider marks). The help the schema gives each key is detent's;
//! here a group says something under itself only where a choice is unclear without it, or what
//! the computer lacks for it (a dictionary, a trusted logo root).
//!
//! The trusted senders are a page of their own, as System Settings pushes a list it keeps: a row
//! per sender with its Remove. The pages are quire's `PaneStack`, its path this page's own.

use super::keys::{KeyRow, value_of};
use crate::settings::{BrandLogos, LoadRemoteImages, Spelling};
use crate::ui::appearance::WindowDirs;
use crate::ui::compose::dictionaries;
use crate::ui::copy;
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
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
}

/// A group of General: its header, and its rows in the order drawn.
pub(super) struct Section {
    pub title: &'static str,
    pub lines: &'static [Line],
}

/// General, group by group.
pub(super) const SECTIONS: [Section; 2] = [
    Section {
        title: copy::headers::READING,
        lines: &[
            Line::Key("reading.remote_images"),
            Line::TrustedSenders,
            Line::Key("reading.brand_logos"),
        ],
    },
    Section {
        title: copy::headers::MAIL,
        lines: &[
            Line::Key("notifications.new_mail"),
            Line::Key("compose.spelling"),
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

/// The root: each group's rows, and under a group what it needs said.
#[component]
fn Groups(on_trusted: EventHandler<()>) -> Element {
    let settings = crate::ui::prefs::current();
    let values =
        toml::Value::try_from(&settings).unwrap_or_else(|_| toml::Value::Table(toml::Table::new()));
    let mut failed = use_signal(|| None::<String>);
    // Read once, as the page opens: the dictionary directories, and the trusted logo roots.
    let found = use_hook(dictionaries);
    let roots = use_hook(|| {
        try_consume_context::<WindowDirs>()
            .map_or(0, |dirs| mail_core::bimi::anchors(&dirs.config).len())
    });
    let onedit = move |(path, value): (String, toml::Value)| {
        failed.set(crate::ui::prefs::change_key(&path, value).err());
    };
    // Under Reading, that no logo can be verified yet; under Mail, that the system has no
    // dictionary for a language, and what Search server does.
    let footer_of = |title: &str| -> Option<String> {
        if title == copy::headers::READING {
            (settings.reading.brand_logos == BrandLogos::On && roots == 0)
                .then(|| copy::fields::LOGOS_UNVERIFIED.to_owned())
        } else {
            let missing = match settings.compose.spelling {
                Spelling::On => found.missing(),
                Spelling::Off => None,
            };
            let said: Vec<String> = missing
                .into_iter()
                .chain([copy::fields::SEARCH_SERVER_MEANS.to_owned()])
                .collect();
            Some(said.join(" "))
        }
    };
    rsx! {
        Form {
            // A change that could not be kept is the page's to say, above every group.
            if let Some(why) = failed() {
                InlineBanner { severity: Severity::Danger, text: copy::errors::NOT_KEPT, detail: Some(TextLine::from(why)) }
            }
            for section in SECTIONS.iter() {
                FormSection { key: "{section.title}", title: Some(section.title.to_owned()), footer: footer_of(section.title),
                    for line in section.lines.iter().copied() {
                        match line {
                            Line::Key(path) => match key_at(path) {
                                Some(spec) => rsx! {
                                    KeyRow { key: "{path}", value: value_of(&values, &spec), spec, onedit }
                                },
                                None => rsx! {},
                            },
                            Line::TrustedSenders => rsx! {
                                List::<&'static str> {
                                    label: copy::titles::TRUSTED_SENDERS,
                                    style: ListStyle::Grouped,
                                    items: vec![ListItem::row(
                                        "trusted",
                                        copy::titles::TRUSTED_SENDERS,
                                        rsx! {
                                            Row {
                                                title: copy::titles::TRUSTED_SENDERS,
                                                size: RowSize::Settings,
                                                accessory: Accessory::Chevron,
                                                onclick: move |_| on_trusted.call(()),
                                                common: Common { id: Some(TRUSTED_ROW.to_owned()), ..Common::default() },
                                            }
                                        },
                                    )],
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// The senders whose images load, a row each with its Remove; with none yet, how one comes to
/// be, under the group.
#[component]
fn TrustedSenders() -> Element {
    let settings = crate::ui::prefs::current();
    let senders = settings.reading.trusted_image_senders.clone();
    let mut failed = use_signal(|| None::<String>);
    let none = senders.is_empty();
    rsx! {
        Form {
            if let Some(why) = failed() {
                InlineBanner { severity: Severity::Danger, text: copy::errors::NOT_KEPT, detail: Some(TextLine::from(why)) }
            }
            FormSection {
                footer: (none && settings.reading.remote_images != LoadRemoteImages::Always)
                    .then(|| copy::fields::HOW_TO_TRUST.to_owned()),
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
    }
}
