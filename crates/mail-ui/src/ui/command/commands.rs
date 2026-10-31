//! The commands ⌘K offers: one type each, with its row's key, words and glyph, the standard
//! action whose chord is drawn beside it, and what it does.
//!
//! A command is found again by its key, never by its words: renaming one changes no behaviour.
//! The panel, the sidebar's foot menu and the menu bar read the same `&dyn Command`, so one
//! command is one place. Only verbs are here. A place is found by its own name
//! ([`super::sections::places_for`]), and a page of Settings, the theme or the state of the
//! connections are Settings' own, one row away.

use super::close;
use crate::ui::bin::Bin;
use crate::ui::copy::{menus, status, toasts};
use crate::ui::motion::{Follow, tell};
use crate::ui::ops::{Composes, Writes, open_composer};
use crate::ui::view::{Bar, BarListing, Shell, Source};
use chordkit::StandardAction;
use dioxus::prelude::*;
use ds::prelude::Icon;
use mail_domain::ThreadId;
use mail_logic::mailbox_filter::for_role;

/// What a command acts on: the window's shell, the list's pages and whether the sidebar is
/// hidden.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Acts {
    pub shell: Signal<Shell>,
    pub pages: Signal<u32>,
    pub side_hidden: Signal<bool>,
}

/// A command the window offers.
pub(in crate::ui) trait Command: Sync + std::fmt::Debug {
    /// The row's key after `action:`. Stable, and never shown.
    fn key(&self) -> &'static str;

    /// Its words, which the matcher compares and the row shows.
    fn label(&self) -> &'static str;

    /// Its glyph.
    fn icon(&self) -> Icon;

    /// The standard action whose chord is drawn beside it.
    fn standard(&self) -> Option<StandardAction> {
        None
    }

    /// Whether the panel is closed before [`Command::run`]. A command that closes it itself, at
    /// a moment of its own, or that keeps it open, says no.
    fn closes_first(&self) -> bool {
        true
    }

    /// Do it.
    fn run(&self, acts: Acts);
}

/// Two commands are one when they have one key.
impl PartialEq for dyn Command {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for dyn Command {}

/// The commands the window can run today, in the order the panel offers them.
pub(in crate::ui) const COMMANDS: &[&dyn Command] = &[
    &Compose,
    &FromTemplate,
    &ForwardAttached,
    &ReplyList,
    &Redirect,
    &EditAsNew,
    &ApplyRules,
    &Print,
    &SyncNow,
    &EmptyTrash,
    &EmptySpam,
    &TidyUp,
    &Sidebar,
    &Conversations,
    &AddAccount,
    &Import,
    &Export,
    &OpenContacts,
    &ImportContacts,
    &ExportContacts,
    &NewView,
    &Settings,
];

/// Run the conversation open behind the menu through `then`, as Reply's `r` takes it; with none
/// open, say so.
fn on_open(shell: Signal<Shell>, then: impl FnOnce(ThreadId)) {
    let Some(open) = shell.peek().open else {
        tell(toasts::OPEN_ONE_FIRST.to_owned(), Follow::Nothing);
        return;
    };
    then(open);
}

/// Empty the bin: go to it first, so what is being emptied is what the window shows behind the
/// sheet that asks.
fn empty(acts: Acts, bin: Bin) {
    let Acts { shell, pages, .. } = acts;
    let source = Source::Mail(for_role(bin.role()));
    let index = shell
        .read()
        .places
        .iter()
        .position(|one| one.source == source);
    if let Some(index) = index {
        crate::ui::sidebar::go_to(shell, pages, index);
        let client = crate::ui::live::client();
        crate::ui::destroy::ask_everything(&client, shell);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct Compose;

impl Command for Compose {
    fn key(&self) -> &'static str {
        "compose"
    }

    fn label(&self) -> &'static str {
        menus::NEW_MESSAGE
    }

    fn icon(&self) -> Icon {
        Icon::Pen
    }

    fn standard(&self) -> Option<StandardAction> {
        Some(StandardAction::New)
    }

    fn closes_first(&self) -> bool {
        false
    }

    fn run(&self, acts: Acts) {
        let client = crate::ui::live::client();
        // A refusal is the composer's to say (`open_composer` logs it).
        let _ = open_composer(&client, acts.shell, Writes::New);
        close(acts.shell);
    }
}

/// Lists the templates in the same panel rather than running anything.
#[derive(Debug)]
pub(in crate::ui) struct FromTemplate;

impl Command for FromTemplate {
    fn key(&self) -> &'static str {
        "from-template"
    }

    fn label(&self) -> &'static str {
        menus::NEW_FROM_TEMPLATE
    }

    fn icon(&self) -> Icon {
        Icon::FilePen
    }

    fn closes_first(&self) -> bool {
        false
    }

    fn run(&self, acts: Acts) {
        let mut shell = acts.shell;
        if let Bar::Open(open) = &mut shell.write().bar {
            open.listing = BarListing::Templates(String::new());
            open.active = 0;
        }
    }
}

#[derive(Debug)]
pub(in crate::ui) struct ForwardAttached;

impl Command for ForwardAttached {
    fn key(&self) -> &'static str {
        "forward-attached"
    }

    fn label(&self) -> &'static str {
        menus::FORWARD_AS_ATTACHMENT
    }

    fn icon(&self) -> Icon {
        Icon::Forward
    }

    /// The conversation open behind the menu, as Forward's `f` takes it. A refusal (no body yet,
    /// or a message rebuilt from its parts) is said where every other outcome of a command is.
    fn run(&self, acts: Acts) {
        let shell = acts.shell;
        let Some(open) = shell.peek().open else {
            tell(toasts::OPEN_ONE_TO_FORWARD.to_owned(), Follow::Nothing);
            return;
        };
        let client = crate::ui::live::client();
        let writes = Writes::answering(&shell.peek(), open, Composes::ForwardAttached);
        if let Err(why) = open_composer(&client, shell, writes) {
            tell(toasts::not_forwarded_as_attachment(&why), Follow::Nothing);
        }
    }
}

/// The open conversation's newest message: answered at its list's address.
#[derive(Debug)]
pub(in crate::ui) struct ReplyList;

impl Command for ReplyList {
    fn key(&self) -> &'static str {
        "reply-list"
    }

    fn label(&self) -> &'static str {
        menus::REPLY_TO_LIST
    }

    fn icon(&self) -> Icon {
        Icon::Group
    }

    fn run(&self, acts: Acts) {
        on_open(acts.shell, |open| {
            crate::ui::reading::compose_on(acts.shell, open, Composes::ReplyList);
        });
    }
}

/// The open conversation's newest message, passed on unchanged.
#[derive(Debug)]
pub(in crate::ui) struct Redirect;

impl Command for Redirect {
    fn key(&self) -> &'static str {
        "redirect"
    }

    fn label(&self) -> &'static str {
        menus::REDIRECT
    }

    fn icon(&self) -> Icon {
        Icon::ArrowUpRight
    }

    /// Its field hangs from the reader's ⋯, as the ⋯'s own Redirect… does.
    fn run(&self, acts: Acts) {
        let mut shell = acts.shell;
        on_open(acts.shell, |open| {
            shell.write().toggle_further(crate::ui::view::FurtherMenu {
                thread: open,
                further: crate::ui::view::Further::Redirect,
                opened: crate::ui::view::Opened::Reader,
            });
        });
    }
}

/// The open conversation's newest message, copied into a new message.
#[derive(Debug)]
pub(in crate::ui) struct EditAsNew;

impl Command for EditAsNew {
    fn key(&self) -> &'static str {
        "edit-as-new"
    }

    fn label(&self) -> &'static str {
        menus::EDIT_AS_NEW
    }

    fn icon(&self) -> Icon {
        Icon::FilePen
    }

    fn run(&self, acts: Acts) {
        on_open(acts.shell, |open| {
            crate::ui::reading::compose_on(acts.shell, open, Composes::Again);
        });
    }
}

/// Run the rules over the conversations picked, or the open one.
#[derive(Debug)]
pub(in crate::ui) struct ApplyRules;

impl Command for ApplyRules {
    fn key(&self) -> &'static str {
        "apply-rules"
    }

    fn label(&self) -> &'static str {
        menus::APPLY_RULES
    }

    fn icon(&self) -> Icon {
        Icon::Check
    }

    fn run(&self, acts: Acts) {
        let shell = acts.shell;
        let targets = shell.peek().acted_on(&crate::ui::picks::drawn_order());
        if targets.is_empty() {
            tell(
                status::NOTHING_TO_APPLY_RULES_TO.to_owned(),
                Follow::Nothing,
            );
            return;
        }
        let client = crate::ui::live::client();
        crate::ui::rules::apply::apply_or_ask(&client, shell, targets);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct Print;

impl Command for Print {
    fn key(&self) -> &'static str {
        "print"
    }

    fn label(&self) -> &'static str {
        menus::PRINT_CONVERSATION
    }

    fn icon(&self) -> Icon {
        Icon::Printer
    }

    fn standard(&self) -> Option<StandardAction> {
        Some(StandardAction::Print)
    }

    fn run(&self, acts: Acts) {
        let open = acts.shell.read().open;
        if let Some(job) = crate::ui::print::job_for(open) {
            crate::ui::print::print(job);
        }
    }
}

#[derive(Debug)]
pub(in crate::ui) struct SyncNow;

impl Command for SyncNow {
    fn key(&self) -> &'static str {
        "sync"
    }

    fn label(&self) -> &'static str {
        menus::SYNC_NOW
    }

    fn icon(&self) -> Icon {
        Icon::Refresh
    }

    fn closes_first(&self) -> bool {
        false
    }

    fn run(&self, acts: Acts) {
        crate::ui::fetching::sync_now(&acts.shell.read());
        close(acts.shell);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct EmptyTrash;

impl Command for EmptyTrash {
    fn key(&self) -> &'static str {
        "empty-trash"
    }

    fn label(&self) -> &'static str {
        menus::EMPTY_TRASH
    }

    fn icon(&self) -> Icon {
        Icon::Command
    }

    fn run(&self, acts: Acts) {
        empty(acts, Bin::Trash);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct EmptySpam;

impl Command for EmptySpam {
    fn key(&self) -> &'static str {
        "empty-spam"
    }

    fn label(&self) -> &'static str {
        menus::EMPTY_SPAM
    }

    fn icon(&self) -> Icon {
        Icon::Command
    }

    fn run(&self, acts: Acts) {
        empty(acts, Bin::Spam);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct TidyUp;

impl Command for TidyUp {
    fn key(&self) -> &'static str {
        "tidy-up"
    }

    fn label(&self) -> &'static str {
        menus::TIDY_UP
    }

    fn icon(&self) -> Icon {
        Icon::Archive
    }

    fn run(&self, acts: Acts) {
        crate::ui::tidy::open(acts.shell);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct Sidebar;

impl Command for Sidebar {
    fn key(&self) -> &'static str {
        "sidebar"
    }

    fn label(&self) -> &'static str {
        menus::HIDE_SIDEBAR
    }

    fn icon(&self) -> Icon {
        Icon::PanelLeft
    }

    fn standard(&self) -> Option<StandardAction> {
        Some(StandardAction::ToggleSidebar)
    }

    fn closes_first(&self) -> bool {
        false
    }

    fn run(&self, acts: Acts) {
        let mut side_hidden = acts.side_hidden;
        side_hidden.set(!side_hidden());
        close(acts.shell);
    }
}

/// Organise by Conversation, turned for the place shown: a menu bar's View menu has it.
#[derive(Debug)]
pub(in crate::ui) struct Conversations;

impl Command for Conversations {
    fn key(&self) -> &'static str {
        "conversations"
    }

    fn label(&self) -> &'static str {
        menus::ORGANISE_BY_CONVERSATION
    }

    fn icon(&self) -> Icon {
        Icon::Group
    }

    fn run(&self, acts: Acts) {
        crate::ui::organise::turn(acts.shell);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct AddAccount;

impl Command for AddAccount {
    fn key(&self) -> &'static str {
        "add-account"
    }

    fn label(&self) -> &'static str {
        menus::ADD_ACCOUNT
    }

    fn icon(&self) -> Icon {
        Icon::Plus
    }

    fn run(&self, _acts: Acts) {
        crate::ui::add_account::open();
    }
}

#[derive(Debug)]
pub(in crate::ui) struct Import;

impl Command for Import {
    fn key(&self) -> &'static str {
        "import"
    }

    fn label(&self) -> &'static str {
        menus::IMPORT_MAIL
    }

    fn icon(&self) -> Icon {
        Icon::Plus
    }

    fn run(&self, acts: Acts) {
        crate::ui::files::open_import(acts.shell);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct Export;

impl Command for Export {
    fn key(&self) -> &'static str {
        "export"
    }

    fn label(&self) -> &'static str {
        menus::EXPORT_MAIL
    }

    fn icon(&self) -> Icon {
        Icon::Forward
    }

    /// The panel is closed first: the sheet opens on the search the window is showing, not on
    /// what was typed into this menu.
    fn run(&self, acts: Acts) {
        crate::ui::files::open_export(acts.shell);
    }
}

/// Settings › Contacts, the book itself: it has no page of its own.
#[derive(Debug)]
pub(in crate::ui) struct OpenContacts;

impl Command for OpenContacts {
    fn key(&self) -> &'static str {
        "contacts"
    }

    fn label(&self) -> &'static str {
        menus::OPEN_CONTACTS
    }

    fn icon(&self) -> Icon {
        Icon::Group
    }

    fn run(&self, _acts: Acts) {
        crate::ui::settings_window::open_at(crate::ui::view::PageId::CONTACTS);
    }
}

/// Contacts in as vCard.
#[derive(Debug)]
pub(in crate::ui) struct ImportContacts;

impl Command for ImportContacts {
    fn key(&self) -> &'static str {
        "import-contacts"
    }

    fn label(&self) -> &'static str {
        menus::IMPORT_CONTACTS
    }

    fn icon(&self) -> Icon {
        Icon::Group
    }

    fn run(&self, _acts: Acts) {
        crate::ui::contacts::import_cards();
    }
}

/// Contacts out as vCard.
#[derive(Debug)]
pub(in crate::ui) struct ExportContacts;

impl Command for ExportContacts {
    fn key(&self) -> &'static str {
        "export-contacts"
    }

    fn label(&self) -> &'static str {
        menus::EXPORT_CONTACTS
    }

    fn icon(&self) -> Icon {
        Icon::Group
    }

    fn run(&self, _acts: Acts) {
        crate::ui::contacts::export_cards();
    }
}

#[derive(Debug)]
pub(in crate::ui) struct NewView;

impl Command for NewView {
    fn key(&self) -> &'static str {
        "new-view"
    }

    fn label(&self) -> &'static str {
        menus::NEW_VIEW
    }

    fn icon(&self) -> Icon {
        Icon::Search
    }

    /// The panel is closed first, like Export's: the view starts from the search the window
    /// shows.
    fn run(&self, acts: Acts) {
        let search = acts.shell.peek().search.clone();
        crate::ui::views::open_new(acts.shell, &search);
    }
}

#[derive(Debug)]
pub(in crate::ui) struct Settings;

impl Command for Settings {
    fn key(&self) -> &'static str {
        "settings"
    }

    fn label(&self) -> &'static str {
        menus::SETTINGS
    }

    fn icon(&self) -> Icon {
        Icon::Settings
    }

    fn run(&self, _acts: Acts) {
        crate::ui::settings_window::open();
    }
}
