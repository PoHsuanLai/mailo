//! The pages of Settings: one type for each, in the order the sidebar lists them.
//!
//! A page says what it is called, its icon and what it draws. Six of them draw mailo's settings
//! schema, each as the groups of rows [`Page::sections`] names, and take the default
//! [`Page::body`]; the others draw a page of their own.

use super::pages::{self, Line, Section};
use crate::ui::copy;
use crate::ui::view::{PageId, Shell};
use dioxus::prelude::*;
use ds::prelude::*;

/// A page of Settings.
pub(in crate::ui) trait Page: Sync {
    /// The id it goes by in the window and in a deep link.
    fn id(&self) -> PageId;

    /// The page's name in the sidebar, which is also the row's accessible name.
    fn name(&self) -> &'static str;

    fn icon(&self) -> Icon;

    /// The groups of rows a page that draws the schema draws, and none for a page that draws its
    /// own.
    fn sections(&self) -> &'static [Section] {
        &[]
    }

    /// What the page draws: the schema's groups, unless it draws its own.
    fn body(&self, _shell: Signal<Shell>) -> Element {
        rsx! { pages::Page { key: "{self.name()}", name: self.name(), sections: self.sections() } }
    }
}

/// Every page, in the order the sidebar lists them.
pub(in crate::ui) const PAGES: &[&dyn Page] = &[
    &General,
    &Accounts,
    &Contacts,
    &Signatures,
    &Spaces,
    &Composing,
    &Viewing,
    &Notifications,
    &Privacy,
    &JunkMail,
    &Rules,
    &Vacation,
    &Keyboard,
];

struct General;

impl Page for General {
    fn id(&self) -> PageId {
        PageId::GENERAL
    }

    fn name(&self) -> &'static str {
        copy::titles::GENERAL
    }

    fn icon(&self) -> Icon {
        Icon::Settings
    }

    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: Some(copy::headers::MAIL),
                lines: &[
                    Line::DefaultMail,
                    Line::Key(crate::settings::WHILE_QUIT),
                    Line::Key("search.server_automatically"),
                ],
            },
            Section {
                title: Some(copy::headers::MAILBOXES),
                lines: &[
                    Line::Key("mailboxes.delete"),
                    Line::Key("mailboxes.erase_trash"),
                    Line::Key("mailboxes.erase_spam"),
                ],
            },
        ]
    }
}

struct Accounts;

impl Page for Accounts {
    fn id(&self) -> PageId {
        PageId::ACCOUNTS
    }

    fn name(&self) -> &'static str {
        copy::titles::ACCOUNTS
    }

    fn icon(&self) -> Icon {
        Icon::Mail
    }

    fn body(&self, shell: Signal<Shell>) -> Element {
        rsx! { super::accounts::Accounts { shell } }
    }
}

struct Contacts;

impl Page for Contacts {
    fn id(&self) -> PageId {
        PageId::CONTACTS
    }

    fn name(&self) -> &'static str {
        copy::titles::CONTACTS
    }

    fn icon(&self) -> Icon {
        Icon::Group
    }

    fn body(&self, shell: Signal<Shell>) -> Element {
        rsx! { crate::ui::contacts::page::Contacts { shell } }
    }
}

struct Signatures;

impl Page for Signatures {
    fn id(&self) -> PageId {
        PageId::SIGNATURES
    }

    fn name(&self) -> &'static str {
        copy::titles::SIGNATURES
    }

    fn icon(&self) -> Icon {
        Icon::Pen
    }

    fn body(&self, shell: Signal<Shell>) -> Element {
        rsx! { super::signatures::Signatures { shell } }
    }
}

struct Spaces;

impl Page for Spaces {
    fn id(&self) -> PageId {
        PageId::SPACES
    }

    fn name(&self) -> &'static str {
        copy::titles::SPACES
    }

    fn icon(&self) -> Icon {
        Icon::Grid
    }

    fn body(&self, shell: Signal<Shell>) -> Element {
        match try_consume_context::<crate::ui::space::Handle>() {
            Some(handle) => rsx! { super::spaces::Spaces { shell, handle } },
            None => rsx! {},
        }
    }
}

struct Composing;

impl Page for Composing {
    fn id(&self) -> PageId {
        PageId::COMPOSING
    }

    fn name(&self) -> &'static str {
        copy::titles::COMPOSING
    }

    fn icon(&self) -> Icon {
        Icon::Pen
    }

    fn sections(&self) -> &'static [Section] {
        &[Section {
            title: None,
            lines: &[
                Line::Key("compose.format"),
                Line::Key("compose.reply_same_format"),
                Line::Key("compose.undo_send"),
                Line::Key("compose.spelling"),
                Line::Key("compose.copy_myself"),
                Line::Key("compose.signature_above_quote"),
            ],
        }]
    }
}

struct Viewing;

impl Page for Viewing {
    fn id(&self) -> PageId {
        PageId::VIEWING
    }

    fn name(&self) -> &'static str {
        copy::titles::VIEWING
    }

    fn icon(&self) -> Icon {
        Icon::Image
    }

    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: None,
                lines: &[
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
                title: Some(copy::headers::MESSAGE_LIST),
                lines: &[
                    Line::Key("window.snippets"),
                    Line::Key("window.label_chips"),
                    Line::Key("window.group_by_date"),
                ],
            },
        ]
    }
}

struct Notifications;

impl Page for Notifications {
    fn id(&self) -> PageId {
        PageId::NOTIFICATIONS
    }

    fn name(&self) -> &'static str {
        copy::titles::NOTIFICATIONS
    }

    fn icon(&self) -> Icon {
        Icon::Bell
    }

    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: None,
                lines: &[
                    Line::Key("notifications.new_mail"),
                    Line::Key("notifications.notify_for"),
                    Line::Vips,
                    Line::Key("notifications.places"),
                    Line::Key("notifications.sound"),
                    Line::Key("notifications.badge"),
                ],
            },
            Section {
                title: Some(copy::headers::NOTIFY_FOR_ACCOUNTS),
                lines: &[Line::MutedAccounts],
            },
        ]
    }
}

struct Privacy;

impl Page for Privacy {
    fn id(&self) -> PageId {
        PageId::PRIVACY
    }

    fn name(&self) -> &'static str {
        copy::titles::PRIVACY
    }

    fn icon(&self) -> Icon {
        Icon::Lock
    }

    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: None,
                lines: &[
                    Line::Key("reading.remote_images"),
                    Line::TrustedSenders,
                    Line::Key("reading.read_receipts"),
                ],
            },
            Section {
                title: Some(copy::headers::REMOTE_CONTENT),
                lines: &[
                    Line::Key("privacy.protect"),
                    Line::Key("privacy.relay_kind"),
                    Line::Address("privacy.relay"),
                    Line::RelayKey,
                    Line::Address("privacy.proxy"),
                ],
            },
        ]
    }
}

struct JunkMail;

impl Page for JunkMail {
    fn id(&self) -> PageId {
        PageId::JUNK_MAIL
    }

    fn name(&self) -> &'static str {
        copy::titles::JUNK_MAIL
    }

    fn icon(&self) -> Icon {
        Icon::OctagonAlert
    }

    fn sections(&self) -> &'static [Section] {
        &[
            Section {
                title: None,
                lines: &[Line::Key("junk.filter"), Line::Key("junk.treat")],
            },
            Section {
                title: Some(copy::headers::NEVER_JUNK_MAIL_FROM),
                lines: &[
                    Line::Key("junk.spare_contacts"),
                    Line::Key("junk.spare_written_to"),
                    Line::Key("junk.spare_full_name"),
                ],
            },
            Section {
                title: None,
                lines: &[Line::Key("junk.trust_server"), Line::Key("junk.order")],
            },
        ]
    }
}

struct Rules;

impl Page for Rules {
    fn id(&self) -> PageId {
        PageId::RULES
    }

    fn name(&self) -> &'static str {
        copy::titles::RULES
    }

    fn icon(&self) -> Icon {
        Icon::FolderInput
    }

    fn body(&self, shell: Signal<Shell>) -> Element {
        rsx! { crate::ui::rules::RulesPage { shell } }
    }
}

struct Vacation;

impl Page for Vacation {
    fn id(&self) -> PageId {
        PageId::VACATION
    }

    fn name(&self) -> &'static str {
        copy::titles::VACATION_REPLY
    }

    fn icon(&self) -> Icon {
        Icon::Sun
    }

    fn body(&self, shell: Signal<Shell>) -> Element {
        rsx! { crate::ui::rules::VacationPage { shell } }
    }
}

struct Keyboard;

impl Page for Keyboard {
    fn id(&self) -> PageId {
        PageId::KEYBOARD
    }

    fn name(&self) -> &'static str {
        copy::titles::KEYBOARD
    }

    fn icon(&self) -> Icon {
        Icon::Keyboard
    }

    fn body(&self, shell: Signal<Shell>) -> Element {
        rsx! { crate::ui::keyboard::KeyboardPage { shell } }
    }
}
