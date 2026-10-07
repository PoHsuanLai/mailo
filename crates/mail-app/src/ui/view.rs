//! What the shell shows, as data.
//!
//! Deliberately free of Dioxus. Which view is selected, what query that means, which thread is
//! open and what the reader should do with a body are all decisions that can be wrong, and none
//! of them needs a window to be wrong in. The rendering layer reads this and draws it.

use crate::ui::selection::{Click, Picked, Toward};
use ds::prelude::*;
use ds::style::appearance::peek::PeekMode;
use mail_core::compose::addresses::{join_addresses, parse_addresses};
use mail_core::place::{pending_snooze, place_filter};
use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};
use porter_core::AccountId;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

/// An entry in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    /// What this place lists. Not a `Filter`: see [`Source`].
    pub source: Source,
    /// Shown as a badge. `None` until counted, which is not the same as zero.
    pub unread: Option<u64>,
}

/// The default sidebar.
///
/// Inbox is `Filter::InMailbox(Inbox)` rather than anything special, which is the plan's claim
/// that a place is just a saved filter — made true here rather than asserted.
/// Where a sidebar place gets its rows.
///
/// Two variants because drafts are genuinely not mail yet. A draft has no thread, no server
/// address and no mailbox — it lives in its own table — so there is no `Filter` that selects
/// one, and a `Drafts` place built from `Filter::InMailbox(MailboxRole::Drafts)` lists nothing,
/// for ever, with no error. That was the state of this sidebar until the composer gave it
/// something to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Threads matching a filter.
    Mail(Filter),
    /// The drafts table.
    Drafts,
    /// A view the user saved: its filter, and how its list is grouped and what its rows offer.
    ///
    /// The whole [`View`] rather than its filter, because the rest of it is what makes it a
    /// view: [`Shell::grouping`] and [`hover_in`] read it while it is the place shown.
    Saved(Box<View>),
    /// The conversations with a follow-up reminder, soonest due first ([`mail_core::follow_up`]).
    ///
    /// Not a `Filter`, for the reason drafts are not: whether a reminder still stands is decided
    /// on the conversation's messages and the user's own addresses, and the domain's filters see
    /// only the summary. The store lists them ([`mail_store::Store::follow_ups`]).
    Waiting,
}

fn source_for(role: MailboxRole) -> Source {
    match role {
        MailboxRole::Drafts => Source::Drafts,
        other => Source::Mail(place_filter(other)),
    }
}

/// What Mute does to a conversation, given what it is now: mute it, or unmute it if it is muted.
///
/// `Op::SetMute` carries the state it sets, so `op_for` cannot produce it; the conversation's own
/// state decides the direction.
pub fn mute_op(summary: &ThreadSummary) -> Op {
    Op::SetMute(match summary.mute {
        Mute::Muted => Mute::Unmuted,
        Mute::Unmuted => Mute::Muted,
    })
}

/// The one mute a gesture applies to several conversations: unmute when every one of them is
/// muted, mute otherwise. One operation for all, never a toggle each, as star and read on a
/// selection are.
pub fn mute_for_all(summaries: &[ThreadSummary]) -> Mute {
    if !summaries.is_empty() && summaries.iter().all(|s| s.mute == Mute::Muted) {
        Mute::Unmuted
    } else {
        Mute::Muted
    }
}

/// What a Mute button or menu item says for these conversations: what pressing it would do.
pub fn mute_label(summaries: &[ThreadSummary]) -> &'static str {
    match mute_for_all(summaries) {
        Mute::Muted => "Mute",
        Mute::Unmuted => "Unmute",
    }
}

pub fn default_places() -> Vec<Place> {
    // Starred sits with the places that ask "what is this mail", ahead of the folders.
    // Snoozed stays with them: a conversation that was put off is looked for there, not beside
    // Trash. Pinned threads stay a place of their own, distinct from a Space's pinned people.
    [
        ("Inbox", source_for(MailboxRole::Inbox)),
        ("Starred", Source::Mail(Filter::Starred(Star::Starred))),
        (
            "Snoozed",
            // Only the ones still away. A due thread is back in the inbox, and showing it here
            // as well would make "snoozed" mean two different things in two places.
            Source::Mail(pending_snooze()),
        ),
        ("Archive", source_for(MailboxRole::Archive)),
        ("Sent", source_for(MailboxRole::Sent)),
        ("Drafts", source_for(MailboxRole::Drafts)),
        ("Spam", source_for(MailboxRole::Spam)),
        ("Trash", source_for(MailboxRole::Trash)),
        ("Pinned", Source::Mail(Filter::Pinned)),
        // Last, so every place before it keeps the index it had: conversations the user is
        // waiting on an answer to, which come back to the inbox if none arrives.
        ("Waiting", Source::Waiting),
    ]
    .into_iter()
    .map(|(name, source)| Place {
        name: name.to_owned(),
        source,
        unread: None,
    })
    .collect()
}

/// A place that lists one label, as opposed to a mailbox.
pub fn is_label_place(place: &Place) -> bool {
    matches!(place.source, Source::Mail(Filter::HasLabel(_)))
}

/// The filter a server folder's place lists: the account's threads with a message the server
/// holds at exactly `path`.
///
/// For a folder with no role and no label behind it. The inbox and the role places list by
/// role, and where folders are labels (Gmail) a folder is listed through its label.
pub fn folder_filter(mailbox: &MailboxRef) -> Filter {
    Filter::And(vec![
        Filter::Account(mailbox.account.clone()),
        Filter::InFolder(mailbox.clone()),
    ])
}

/// The folder a place lists, when it is a server folder's place.
pub fn folder_of(place: &Place) -> Option<&MailboxRef> {
    match &place.source {
        Source::Mail(Filter::And(parts)) => match parts.as_slice() {
            [Filter::Account(account), Filter::InFolder(mailbox)]
                if *account == mailbox.account =>
            {
                Some(mailbox)
            }
            _ => None,
        },
        _ => None,
    }
}

/// The sidebar: the default places, then one per label, then one per server folder, then one
/// per saved view, in that order — which is also the order the badges are counted in, index for
/// index.
///
/// A folder's place is named by its last level, which is what its row and the list's title say.
/// Saved views come last so that keeping or forgetting one moves no other place's index.
pub fn places_with(
    labels: &[(String, LabelId)],
    folders: &[(String, MailboxRef)],
    views: &[View],
) -> Vec<Place> {
    let labelled = labels.iter().map(|(name, id)| Place {
        name: name.clone(),
        source: Source::Mail(Filter::HasLabel(*id)),
        unread: None,
    });
    let foldered = folders.iter().map(|(name, mailbox)| Place {
        name: name.clone(),
        source: Source::Mail(folder_filter(mailbox)),
        unread: None,
    });
    default_places()
        .into_iter()
        .chain(labelled)
        .chain(foldered)
        .chain(views.iter().map(saved_place))
        .collect()
}

/// The sidebar place a saved view is.
pub fn saved_place(view: &View) -> Place {
    Place {
        name: view.name.clone(),
        source: Source::Saved(Box::new(view.clone())),
        unread: None,
    }
}

/// The saved view a place is, when it is one.
pub fn saved_of(place: &Place) -> Option<&View> {
    match &place.source {
        Source::Saved(view) => Some(view),
        Source::Mail(_) | Source::Drafts | Source::Waiting => None,
    }
}

/// What a place's badge counts, or `None` when it has no badge.
///
/// Unread threads, and only for mail: "3 unread drafts" is not a thing, because a draft is not
/// something that arrives and is not something anyone has failed to read yet.
///
/// Sent and Archive get one too. That looks odd until a filter rule files an unread message
/// straight into Archive, at which point a badgeless Archive is a message the user never learns
/// about.
pub fn badge_filter(source: &Source) -> Option<Filter> {
    match source {
        Source::Mail(filter) => Some(Filter::And(vec![
            filter.clone(),
            Filter::Read(ReadState::Unread),
        ])),
        Source::Saved(view) => Some(Filter::And(vec![
            view.filter.clone(),
            Filter::Read(ReadState::Unread),
        ])),
        // Nothing in it is news: the user wrote the last word and is waiting for someone else's.
        Source::Drafts | Source::Waiting => None,
    }
}

/// Which palette a Space resolves to: quire's, with its three states.
pub use ds::prelude::Theme;

/// Whether a provider chip draws the cached icon or the letter.
///
/// Icons are the first-run choice. A chip whose file has not been fetched yet still
/// draws the letter, so the window never waits on the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Copy, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum Marks {
    /// The provider's own icon, once it is cached.
    #[default]
    Icons,
    /// The letter, even when an icon is cached.
    Letters,
}

impl From<crate::settings::ProviderMarks> for Marks {
    fn from(marks: crate::settings::ProviderMarks) -> Self {
        match marks {
            crate::settings::ProviderMarks::Icons => Marks::Icons,
            crate::settings::ProviderMarks::Letters => Marks::Letters,
        }
    }
}

impl From<Marks> for crate::settings::ProviderMarks {
    fn from(marks: Marks) -> Self {
        match marks {
            Marks::Icons => crate::settings::ProviderMarks::Icons,
            Marks::Letters => crate::settings::ProviderMarks::Letters,
        }
    }
}

impl Marks {
    /// The order a picker offers them: their icons, then letters.
    pub const ALL: [Marks; 2] = [Marks::Icons, Marks::Letters];

    /// What the picker calls it.
    pub fn label(self) -> &'static str {
        match self {
            Marks::Icons => "Their icons",
            Marks::Letters => "Letters",
        }
    }

    /// The choice a stored word names, or [`None`] when it is not one of the two.
    pub fn parse(word: &str) -> Option<Marks> {
        match word {
            "icons" => Some(Marks::Icons),
            "letters" => Some(Marks::Letters),
            _ => None,
        }
    }
}

/// The window's own preferences: what is mailo's rather than the design system's.
///
/// Theme, accent and motion are quire's [`ds::Appearance`], in `appearance.toml`
/// (`crate::ui::appearance`); a Space's theme and motion are the Space's. What is left here is
/// mail policy with no place in any quire type: whether a provider chip shows its icon.
///
/// A missing field is the first-run value, and an unknown word is that field's default, not a
/// failure of the whole value. An unknown field is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Copy, Hash, Default)]
#[serde(default)]
pub struct Appearance {
    /// Provider marks: their icons, or the letter.
    #[serde(deserialize_with = "de_marks")]
    pub marks: Marks,
}

/// A stored provider-mark choice. Anything that is not `icons` or `letters` is [`Marks::default`].
fn de_marks<'de, D>(deserializer: D) -> Result<Marks, D::Error>
where
    D: Deserializer<'de>,
{
    let word = String::deserialize(deserializer)?;
    Ok(Marks::parse(&word).unwrap_or_default())
}

/// Where the open reader sits. Per session, and not persisted.
///
/// Side is the grid's third column, and mailo's own: quire's [`PeekMode`] is only the two
/// that float. Floating is a stylesheet change on `div.app` — the reader component stays where
/// it is in the tree, because moving it would reload the sandboxed frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Peek {
    /// The grid's third column.
    #[default]
    Side,
    /// Over the window: a centred panel, or the whole of it.
    Float(PeekMode),
}

impl Peek {
    /// A panel over the middle of the window.
    pub const CENTER: Peek = Peek::Float(PeekMode::Center);
    /// The whole window.
    pub const FULL: Peek = Peek::Float(PeekMode::Full);

    /// `side`, `center` or `full`, written as `data-peek`.
    pub fn slug(self) -> &'static str {
        match self {
            Peek::Side => "side",
            Peek::Float(mode) => mode.slug(),
        }
    }

    /// What the peek button names itself: "Side peek", "Centre peek", "Full page".
    pub fn label(self) -> &'static str {
        match self {
            Peek::Side => "Side peek",
            Peek::Float(mode) => mode.label(),
        }
    }

    /// Centre and full float over the window. Side stays in the grid.
    pub fn floats(self) -> bool {
        !matches!(self, Peek::Side)
    }
}

/// How the loaded page is grouped.
///
/// Client-side, and only the rows already on this page. The store's own grouping is a
/// different feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageGroup {
    #[default]
    None,
    Sender,
    Date,
    Label,
    Unread,
}

/// Whether a row draws one of its parts.
///
/// A closed pair, so a call site cannot pass a bare `true` and leave the reader guessing
/// which way round it went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPart {
    Shown,
    Hidden,
}

/// Which parts of a row the loaded page draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageParts {
    pub snippet: RowPart,
    pub provider: RowPart,
    pub chips: RowPart,
    pub time: RowPart,
}

impl Default for PageParts {
    fn default() -> Self {
        Self {
            snippet: RowPart::Shown,
            provider: RowPart::Shown,
            chips: RowPart::Shown,
            time: RowPart::Shown,
        }
    }
}

impl PageParts {
    /// The part named by a properties-menu key, if it is one.
    pub fn part_mut(&mut self, key: &str) -> Option<&mut RowPart> {
        match key {
            "snippet" => Some(&mut self.snippet),
            "provider" => Some(&mut self.provider),
            "chips" => Some(&mut self.chips),
            "time" => Some(&mut self.time),
            _ => None,
        }
    }
}

impl RowPart {
    /// The other way of showing a part.
    pub fn toggle(self) -> Self {
        match self {
            RowPart::Shown => RowPart::Hidden,
            RowPart::Hidden => RowPart::Shown,
        }
    }

    /// Whether the row should draw this part.
    pub fn shown(self) -> bool {
        matches!(self, RowPart::Shown)
    }
}

/// How the list is grouped: by the page's Group menu, or by the saved view being shown.
///
/// Two vocabularies because they are two features. The menu's [`PageGroup`] is a quick look at
/// the loaded page; a view's [`GroupKey`] is part of what the view was saved as, and says things
/// the menu cannot, such as "tagged Travel, and not".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grouping {
    Page(PageGroup),
    Saved(GroupKey),
}

/// Which list-bar menu is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageMenu {
    #[default]
    Closed,
    Group,
    Properties,
}

/// The search bar's panel of suggestions, under its field in the list's toolbar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Bar {
    /// No panel: the field is a search box and nothing more.
    #[default]
    Closed,
    /// The panel is up.
    Open(BarOpen),
}

/// The panel while it is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BarOpen {
    /// The highlighted suggestion, counted over the rows (the section titles not counted).
    pub active: usize,
    /// What the panel lists.
    pub listing: BarListing,
    /// The search the window showed before the field took the keyboard. What is typed into the
    /// bar is a search and a command's name at once; a command runs on the search there was
    /// before its name was typed, so "Export mail…" exports what the list showed, not "export".
    pub before: String,
}

impl BarOpen {
    /// The panel as it opens over the search `before`: the search's suggestions, the first
    /// highlighted.
    pub fn over(before: String) -> Self {
        Self {
            active: 0,
            listing: BarListing::Search,
            before,
        }
    }
}

/// What the bar's panel lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarListing {
    /// Mail, commands, places and people for the field's text, which is the list's search.
    Search,
    /// The templates, after "New from template", narrowed by the text held here: while it
    /// lists them the field narrows the templates and leaves the list's search alone.
    Templates(String),
}

/// Which of the two mail-file sheets is open, and what its field holds.
///
/// The field's text lives here because the debounce that reads it takes a function of the
/// shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSheet {
    /// "Import mail…": the path typed so far.
    Import { path: String },
    /// "Export mail…": which messages, as a search or a place's name.
    Export { query: String },
}

/// The Rules page of Settings: which account's rules, vacation reply and server script it
/// shows. `None` is the first account there is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RulesPage {
    pub account: Option<AccountId>,
}

/// The Connection Doctor sheet while it is open. It keeps nothing: it reads every account's
/// link, and a sign-in it starts is the Add account sheet's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoctorSheet;

/// The account sheet while it is open: which account, and how far a removal has got. It keeps
/// nothing of the account itself, which it reads from the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSheet {
    pub account: AccountId,
    pub step: AccountStep,
}

/// A Space's menu, opened by a right click on the Space (its dot, its name, the sidebar's foot)
/// or a click on its name, and what it shows now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceMenu {
    /// Which Space, by index into the window's Spaces.
    pub index: usize,
    /// Where it opened, in the window's client pixels: the menu and each part stand there.
    pub at: (i32, i32),
    pub showing: SpaceShowing,
}

/// The menu itself, or one of the parts that opens at the pointer instead of changing at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceShowing {
    Menu,
    /// A field holding the name.
    Rename,
    /// The colour field, stops, grain and presets (quire's `SpaceColour`).
    Colour,
    /// Asking before the Space goes.
    Delete,
}

/// A page of Settings, in the order its sidebar lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SettingsPage {
    #[default]
    General,
    Accounts,
    Contacts,
    Rules,
    /// OpenPGP keys and S/MIME certificates. Nothing about a key is kept in the shell: the page
    /// reads the store, and a passphrase, a password or a secret key never passes through it.
    Keys,
    Keyboard,
}

/// Where the account sheet is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStep {
    /// The account's settings, and Remove Account.
    Showing,
    /// Asking whether to remove it.
    Asking,
    /// Removing it: the keyring and the database are being written.
    Removing,
    /// It was not removed, and this is why.
    Refused(String),
}

/// The Keyboard page of Settings: which action is waiting for its key, and what the last change
/// came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyboardPage {
    /// The action waiting for its new key: the next press is offered to it, and the window
    /// gives every key to the page until then. `None` is waiting for nothing.
    pub listening: Option<Shortcut>,
    /// Why the last key was not taken, or the keymap not kept, in words, under the row it is
    /// about.
    pub said: Option<KeySaid>,
}

/// Why a change on the Keyboard page did not happen, and which row says so: an action's, or
/// `None` for Reset All's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySaid {
    pub about: Option<Shortcut>,
    pub text: String,
}

/// The attachment viewer while it is open: which stored part of which message, and for a PDF
/// which page, from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewing {
    pub message: MessageId,
    pub index: usize,
    pub page: u32,
    /// How many pages the part has, once one has been drawn; `None` before, and for a picture.
    pub pages: Option<u32>,
}

impl Viewing {
    /// The first page of attachment `index` of `message`.
    pub fn of(message: MessageId, index: usize) -> Self {
        Self {
            message,
            index,
            page: 0,
            pages: None,
        }
    }

    /// `by` pages on, held to the pages there are. Until a page has been drawn the count is not
    /// known and nothing turns forward, so a held key cannot run ahead into pages that do not
    /// exist; a picture never has pages.
    pub fn turned(self, by: i32) -> Self {
        let last = match self.pages {
            Some(pages) => pages.saturating_sub(1),
            None => self.page,
        };
        let page = self.page.saturating_add_signed(by).min(last);
        Self { page, ..self }
    }
}

/// Everything the shell is currently showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub places: Vec<Place>,
    pub selected: usize,
    /// What the user typed in the search box.
    pub search: String,
    pub open: Option<ThreadId>,
    /// Whether the open thread was opened from its Today tab. The sidebar has one selected row:
    /// that tab while this holds, the place otherwise.
    pub from_today: bool,
    /// The conversations picked for an action on several at once. Empty: actions mean `open`.
    ///
    /// Dropped with the place ([`Self::select`]), and replaced by a plain click
    /// ([`Self::open`]). Always read through the list's ids, so a pick that has left the list
    /// is never acted on.
    pub picked: Picked,
    /// Whether "Show images" was pressed for the thread currently open: the reader may fetch
    /// every one of its messages' remote images.
    ///
    /// Per thread and not persisted. Under `reading.remote_images = "ask"` (the default),
    /// consenting to load one sender's images is not consent for the next message, and a remote
    /// image is a read receipt the sender never asked for. `trusted` and `always` are the
    /// person's standing consent, given in the settings rather than here; the reader adds what
    /// they allow message by message (`reading::images`), and this press covers the rest.
    pub show_remote_images: bool,
    /// Where the reader sits. Per session, not persisted.
    pub peek: Peek,
    /// The conversation whose label menu is open, if any.
    ///
    /// One at a time and identified by thread rather than by row index: a sync can land between
    /// the click and the choice, and a menu pinned to "the fourth row" would then be labelling
    /// something else.
    pub labelling: Option<ThreadId>,
    /// The conversation whose snooze menu is open, if any.
    pub snoozing: Option<ThreadId>,
    /// The conversation whose "Move to…" menu is open, if any. By thread, like `labelling`.
    pub filing: Option<ThreadId>,
    /// The composer, when one is open.
    ///
    /// `Option` rather than a `mode` enum on `Shell`: composing does not replace reading, it
    /// sits beside it. A user who opens a reply and then clicks another thread should still
    /// have their half-written reply when they come back, which a mode would have thrown away.
    pub composing: Option<Composing>,
    /// Every account that can send, as `(address, id)`, for the composer's From row.
    ///
    /// Beside `labels` and filled the same way, because it answers the same kind of question:
    /// something the shell needs to know about the world that is a list of values rather than a
    /// connection, so `query` and `apply_to` stay pure functions of what is on screen.
    pub accounts: Vec<(String, AccountId)>,
    /// Every label name the store knows, and which label bears it.
    ///
    /// Data rather than a connection, so `query` stays a pure function of the shell: `label:` is
    /// the one search term that needs the world, and the world arrives as a list. A name can
    /// appear more than once — `UNIQUE (account, name)` is per account, so "travel" on two
    /// accounts is two labels and someone typing the word means both.
    pub labels: Vec<(String, LabelId)>,
    /// How the window looks.
    ///
    /// A choice, held like every other one: the window reads it rather than deciding.
    /// Remembered in the config directory, not in the mail database; a shell built with no
    /// stored choice is [`Appearance::default`].
    pub appearance: Appearance,
    /// The account tile that is pressed. `None` is every account in [`Self::scope`].
    pub account: Option<AccountId>,
    /// Accounts the current Space shows.
    pub scope: crate::ui::space::Scope,
    /// How this page's rows are grouped. Not a store query.
    pub group: PageGroup,
    /// Which parts of a row this page draws.
    pub parts: PageParts,
    /// The list-bar menu that is open.
    pub page_menu: PageMenu,
    /// The search bar's panel of suggestions.
    pub bar: Bar,
    /// What the Contacts page of Settings is filtered by.
    pub contacts: String,
    /// The Import or Export sheet while it is open, with the text its field holds. `None` is
    /// closed.
    pub files: Option<FileSheet>,
    /// Which account the Rules page of Settings shows.
    pub rules: RulesPage,
    /// The Connection Doctor sheet while it is open. `None` is closed.
    pub doctor: Option<DoctorSheet>,
    /// The account sheet, opened from the Connection Doctor or Settings, while it is open. `None`
    /// is closed.
    pub account_sheet: Option<AccountSheet>,
    /// Settings while it is open, on the page shown. `None` is closed.
    pub settings: Option<SettingsPage>,
    /// The saved-view editor while it is open, with what its fields hold. `None` is closed.
    pub view_editor: Option<crate::ui::saved::ViewDraft>,
    /// Which key does what: the shipped keys with the user's own over them, read from
    /// `keyboard.json` when the window opens.
    pub keymap: crate::ui::keymap::Keymap,
    /// The Keyboard page of Settings: the action waiting for its key, and what was said.
    pub keyboard: KeyboardPage,
    /// The Delete forever / Empty Trash confirmation while it is open. `None` is closed.
    pub destroying: Option<crate::ui::bin::Destroying>,
    /// A Space's menu, or the part of it a row opened, at the pointer. `None` is closed.
    pub space_menu: Option<SpaceMenu>,
    /// What the undo toast and ⌘Z can take back, newest last.
    pub undo: mail_core::undo::UndoStack,
    /// The attachment viewer, over the window. `None` is closed. Belongs to the open thread:
    /// [`Self::open`], [`Self::close`] and [`Self::select`] drop it.
    pub viewing: Option<Viewing>,
}

/// A message being edited, as the widgets hold it.
///
/// Recipients are `String`s, not `Vec<Address>`, because that is what a text box contains. The
/// user is mid-typing for most of this struct's life, and a half-typed address is not an
/// `Address` — forcing it to be one means either rejecting every keystroke or inventing a
/// parse that silently discards what was typed. Parsing happens once, on save, in
/// [`parse_addresses`].
///
/// No `Default`: a composer with no draft behind it is not a state this can be in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composing {
    /// The draft this edits. It already exists in the store before the composer opens.
    pub draft: DraftId,
    /// The account this leaves from.
    ///
    /// Held here so the From row can show it and change it. A reply takes it from the message
    /// it answers and it is never in doubt; a new message is the one case where the sender
    /// chooses, and the choice has to be visible or it is not a choice.
    pub from: AccountId,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    /// What the draft carries, as `(name, size)` ready to show.
    ///
    /// Filled by the composer rather than by [`Composing::of`], because the sizes live in the
    /// blob table and this module is deliberately free of the store — every decision in it is
    /// tested without one. A list that is merely stale shows the wrong size for a moment; a
    /// `view` that could read the database would be a `view` nothing could test cheaply.
    pub attachments: Vec<(String, String)>,
    /// What went wrong with the last save or send, shown inline.
    pub notice: Option<String>,
    /// Discard has been asked for once and is waiting to be meant.
    ///
    /// The button sits beside Close, and it now deletes the draft rather than merely closing
    /// the pane, so a mis-aimed click would destroy something that no longer exists anywhere
    /// else. One extra click is the whole of the protection, which is what every client that
    /// cannot offer undo does.
    pub confirming_discard: bool,
}

impl Composing {
    /// Open the composer on an existing draft.
    pub fn of(draft: &Draft) -> Self {
        Self {
            draft: draft.id,
            from: draft.account.clone(),
            to: join_addresses(&draft.to),
            cc: join_addresses(&draft.cc),
            subject: draft.subject.clone(),
            body: draft.text.clone(),
            attachments: Vec::new(),
            notice: None,
            confirming_discard: false,
        }
    }

    /// The edited draft, or what is wrong with it.
    ///
    /// Takes the stored draft rather than building one, so everything the composer does not
    /// show — the identity, the message being replied to, the attachments — survives an edit
    /// instead of being reset to a default the user never chose.
    pub fn apply_to(
        &self,
        base: &Draft,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Draft, String> {
        let to = parse_addresses(&self.to).map_err(|e| format!("To: {e}"))?;
        let cc = parse_addresses(&self.cc).map_err(|e| format!("Cc: {e}"))?;
        Ok(Draft {
            to,
            cc,
            subject: self.subject.clone(),
            text: self.body.clone(),
            updated: now,
            ..base.clone()
        })
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            places: default_places(),
            selected: 0,
            search: String::new(),
            open: None,
            from_today: false,
            picked: Picked::none(),
            show_remote_images: false,
            peek: Peek::Side,
            composing: None,
            labelling: None,
            snoozing: None,
            filing: None,
            accounts: Vec::new(),
            labels: Vec::new(),
            appearance: Appearance::default(),
            account: None,
            scope: crate::ui::space::Scope::All,
            group: PageGroup::None,
            parts: PageParts::default(),
            page_menu: PageMenu::Closed,
            bar: Bar::Closed,
            contacts: String::new(),
            files: None,
            rules: RulesPage::default(),
            view_editor: None,
            keymap: crate::ui::keymap::Keymap::default(),
            keyboard: KeyboardPage::default(),
            doctor: None,
            account_sheet: None,
            settings: None,
            destroying: None,
            space_menu: None,
            undo: mail_core::undo::UndoStack::default(),
            viewing: None,
        }
    }
}

impl Shell {
    /// The query the list pane should run.
    ///
    /// A non-empty search box replaces the place rather than narrowing it, which is what every
    /// mail client does and what users expect: searching while in Archive should not hide
    /// results that live in the Inbox.
    pub fn query(&self, limit: u32) -> Query {
        let needle = self.search.trim();
        let mut sort = Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        };
        let filter = if needle.is_empty() {
            match self.places.get(self.selected).map(|place| &place.source) {
                Some(Source::Mail(filter)) => filter.clone(),
                Some(Source::Saved(view)) => {
                    sort = view.sort;
                    view.filter.clone()
                }
                // Reachable only if a caller asks for a query while Drafts is selected.
                // `listing` is the method that knows the difference; this stays total rather
                // than panicking, and `All` is the least surprising thing to show.
                Some(Source::Drafts) | Some(Source::Waiting) | None => Filter::All,
            }
        } else {
            mail_core::query::parse_with(
                needle,
                &chrono::Local,
                &mail_core::query::named(&self.labels),
            )
        };
        let filter = self.with_account(filter);
        Query {
            filter,
            sort,
            page: PageReq { after: None, limit },
        }
    }

    /// The saved view the list is showing, if it is showing one.
    ///
    /// Not while a search is typed: a search replaces the place (see [`Self::query`]), so the
    /// rows are the search's and the view's grouping and row actions are not theirs.
    pub fn saved_view(&self) -> Option<&View> {
        if !self.search.trim().is_empty() {
            return None;
        }
        self.places.get(self.selected).and_then(saved_of)
    }

    /// How the list is grouped now. The Group menu wins when it has been set to something; left
    /// at None, a saved view's own grouping applies.
    pub fn grouping(&self) -> Grouping {
        match (
            self.group,
            self.saved_view().and_then(|v| v.group_by.clone()),
        ) {
            (PageGroup::None, Some(key)) => Grouping::Saved(key),
            (page, _) => Grouping::Page(page),
        }
    }

    /// What the list pane should show.
    ///
    /// A search box with anything in it always means threads, even while Drafts is selected:
    /// searching is global here, and a user who types into it is looking for a message, not
    /// filtering the drafts they can already see.
    pub fn listing(&self, limit: u32) -> Listing {
        if !self.search.trim().is_empty() {
            return Listing::Threads(self.query(limit));
        }
        match self.places.get(self.selected).map(|p| &p.source) {
            Some(Source::Drafts) => Listing::Drafts,
            Some(Source::Waiting) => Listing::Waiting {
                scope: self.account_filter(),
            },
            Some(Source::Mail(filter)) if *filter == place_filter(MailboxRole::Inbox) => {
                Listing::Inbox {
                    query: self.query(limit),
                    scope: self.account_filter(),
                }
            }
            _ => Listing::Threads(self.query(limit)),
        }
    }

    /// Narrow `filter` to the pressed account tile, or to the Space when it names accounts.
    ///
    /// A tile wins over the Space: pressing one account inside a Space of three shows that
    /// account. No tile and an empty scope leave the filter alone, which is every account.
    fn with_account(&self, filter: Filter) -> Filter {
        match self.account_filter() {
            Some(account) => Filter::And(vec![account, filter]),
            None => filter,
        }
    }

    /// The pressed tile, or the Space's accounts, as a filter. `None` is every account.
    ///
    /// The search pipeline narrows its candidates with this, so a search inside a Space finds
    /// what [`Self::query`] would, and nothing from an account the Space leaves out.
    pub fn account_filter(&self) -> Option<Filter> {
        self.scope.narrowed(self.account.clone()).filter()
    }

    /// Select a place, and drop any open thread that no longer belongs to the new list.
    pub fn select(&mut self, index: usize) {
        if index < self.places.len() {
            self.selected = index;
            self.open = None;
            self.from_today = false;
            // What was picked was picked in the old list.
            self.picked = Picked::none();
            // Consent is per thread, so changing what is shown revokes it.
            self.show_remote_images = false;
            self.viewing = None;
        }
    }

    /// Open a thread. What was picked is replaced by it, as a plain click on a row replaces a
    /// selection in any list, and a Shift range measures from it next.
    pub fn open(&mut self, thread: ThreadId) {
        self.open = Some(thread);
        self.from_today = false;
        self.picked = Picked::clicked(thread);
        self.show_remote_images = false;
        self.viewing = None;
    }

    /// Open a thread from its Today tab: the tab is then the sidebar's selected row.
    pub fn open_from_today(&mut self, thread: ThreadId) {
        self.open(thread);
        self.from_today = true;
    }

    /// Whether place `index` is the sidebar's selected row: the list's place, unless a Today tab
    /// took the selection by opening its thread.
    pub fn place_selected(&self, index: usize) -> bool {
        self.selected == index && !self.from_today
    }

    /// The Today tab that is the sidebar's selected row, if one took it.
    pub fn selected_tab(&self) -> Option<ThreadId> {
        self.open.filter(|_| self.from_today)
    }

    /// Close the reader.
    ///
    /// Consent is per thread, so closing revokes it the way [`Self::open`] and
    /// [`Self::select`] do. A remote image is a read receipt.
    pub fn close(&mut self) {
        self.open = None;
        self.from_today = false;
        self.show_remote_images = false;
        self.viewing = None;
    }

    /// A click on a conversation's row, among the listed `ids`: a plain one opens it, Ctrl
    /// picks it or puts it back, Shift picks the range from the anchor to it.
    pub fn click(&mut self, thread: ThreadId, click: Click, ids: &[ThreadId]) {
        match click {
            Click::Plain => self.open(thread),
            Click::Toggle => self.picked = self.picked.toggle(thread, self.open, ids),
            Click::Range => self.picked = self.picked.range(thread, self.open, ids),
        }
    }

    /// Shift+j or Shift+k among the listed `ids`.
    pub fn extend(&mut self, toward: Toward, ids: &[ThreadId]) {
        self.picked = self.picked.extend(toward, self.open, ids);
    }

    /// Select all: every listed conversation.
    pub fn pick_all(&mut self, ids: &[ThreadId]) {
        self.picked = Picked::all(ids);
    }

    /// Drop the selection, leaving the reader as it is. Returns whether anything listed was
    /// picked, so Esc can mean this before it means closing the reader.
    pub fn unpick(&mut self, ids: &[ThreadId]) -> bool {
        let had = self.picked.any(ids);
        self.picked = Picked::none();
        had
    }

    /// Whether `thread`'s row is drawn selected: picked, or, with nothing picked, open.
    pub fn is_selected(&self, thread: ThreadId, ids: &[ThreadId]) -> bool {
        if self.picked.any(ids) {
            self.picked.holds(thread, ids)
        } else {
            self.open == Some(thread)
        }
    }

    /// The conversations an action from the keyboard means, in list order: the picked ones,
    /// or, with nothing picked, the open one if it is still listed.
    pub fn acted_on(&self, ids: &[ThreadId]) -> Vec<ThreadId> {
        let chosen = self.picked.chosen(ids);
        if chosen.is_empty() {
            self.open
                .filter(|id| ids.contains(id))
                .into_iter()
                .collect()
        } else {
            chosen
        }
    }

    /// The conversations an action on `thread`'s own row means: the whole selection when the
    /// row is part of it, and the row alone when it is not. A press on a row outside the
    /// selection is about that row, as a right-click outside a selection is anywhere else.
    pub fn with_selection(&self, thread: ThreadId, ids: &[ThreadId]) -> Vec<ThreadId> {
        if self.picked.holds(thread, ids) {
            self.picked.chosen(ids)
        } else {
            vec![thread]
        }
    }

    /// Open the composer on `draft`.
    pub fn compose(&mut self, draft: &Draft) {
        self.composing = Some(Composing::of(draft));
    }

    /// Drop the composer's widgets without writing them anywhere.
    ///
    /// This **loses** whatever has not been saved, so the only caller that may reach it without
    /// saving first is an explicit Discard. Closing saves and then calls this; an earlier
    /// version closed straight into it and quietly threw away everything typed since the last
    /// Save, behind a comment claiming that could not happen.
    pub fn close_composer(&mut self) {
        self.composing = None;
    }

    /// The sanitizer policy for the thread currently open, by the press alone.
    pub fn policy(&self) -> SanitizePolicy {
        self.policy_with(false)
    }

    /// The sanitizer policy for one message of the thread currently open: remote images allowed
    /// when "Show images" was pressed, or when the settings allow them for this message
    /// (`allowed`, from `reading::images::auto_allow`).
    pub fn policy_with(&self, allowed: bool) -> SanitizePolicy {
        SanitizePolicy {
            remote_images: if self.show_remote_images || allowed {
                RemoteImages::Allowed
            } else {
                RemoteImages::Blocked
            },
            ..SanitizePolicy::FRAME
        }
    }
}

/// What a row offers for a thread, in its menu (once the hover strip's buttons; the name stays).
///
/// Derived from where the thread is, so Archive does not offer "archive" and Trash offers
/// "restore". Returns [`OpKind`] rather than [`Op`] because a menu item cannot carry a payload
/// that does not exist yet.
pub fn hover_actions(summary: &ThreadSummary) -> Vec<OpKind> {
    let mut out = Vec::new();
    if summary.mailboxes.contains(MailboxRole::Inbox) {
        out.push(OpKind::Archive);
    }
    if summary.mailboxes.contains(MailboxRole::Trash)
        || summary.mailboxes.contains(MailboxRole::Archive)
        || summary.mailboxes.contains(MailboxRole::Spam)
    {
        out.push(OpKind::Restore);
    }
    if !summary.mailboxes.contains(MailboxRole::Trash) {
        out.push(OpKind::Trash);
    }
    out.push(match summary.read {
        ReadState::Unread => OpKind::MarkRead,
        ReadState::Read => OpKind::MarkUnread,
    });
    out.push(match summary.star {
        Star::Unstarred => OpKind::Star,
        Star::Starred => OpKind::Unstar,
    });
    // Always offered. None depends on where the conversation is or what state it is in: a
    // forward is the message being passed on, a pin is a note to yourself about it, a mute is
    // about the replies still to come, and a label is a name you are giving it.
    out.push(OpKind::Pin);
    out.push(OpKind::Mute);
    out.push(OpKind::Snooze);
    out.push(OpKind::AddLabel);
    out.push(OpKind::Forward);
    out
}

/// What a row's menu offers for a thread in `view`: the view's own actions where it names any,
/// else [`hover_actions`].
///
/// A view names kinds, not directions, so its Star or Mark read is drawn as whichever of the
/// pair the conversation needs, and a button the conversation cannot take (Archive on one not in
/// the inbox) is left out, as [`hover_actions`] leaves it out. The keyboard is not narrowed by a
/// view: [`offers`] still asks [`hover_actions`].
pub fn hover_in(view: Option<&View>, summary: &ThreadSummary) -> Vec<OpKind> {
    let offered = hover_actions(summary);
    let Some(view) = view.filter(|view| !view.hover.is_empty()) else {
        return offered;
    };
    let mut out: Vec<OpKind> = Vec::new();
    for kind in &view.hover {
        let kind = match kind {
            OpKind::Star | OpKind::Unstar => match summary.star {
                Star::Unstarred => OpKind::Star,
                Star::Starred => OpKind::Unstar,
            },
            OpKind::MarkRead | OpKind::MarkUnread => match summary.read {
                ReadState::Unread => OpKind::MarkRead,
                ReadState::Read => OpKind::MarkUnread,
            },
            other => *other,
        };
        let allowed = match kind {
            // Neither needs a place to be allowed from; a reply is a draft.
            OpKind::Reply | OpKind::ReplyAll => true,
            other => offers(summary, other),
        };
        if allowed && !out.contains(&kind) {
            out.push(kind);
        }
    }
    out
}

/// What a keystroke means.
///
/// The shell had no keyboard at all: not a key handler anywhere in it, so moving between
/// conversations, opening one, archiving, starring, replying and closing a half-written reply
/// were each a mouse click and nothing else. A mail client is a thing people spend hours a day
/// in, and this is the part of "daily driver" that does not depend on anyone's taste.
///
/// Which key means which is [`crate::ui::keymap`]'s: a table, with the user's own keys over it. The
/// serde form names an action in `keyboard.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shortcut {
    /// Move to the next conversation and open it.
    Next,
    /// Move to the previous one.
    Previous,
    /// Shift+j: pick down the list from the anchor, one more row each press, opening nothing.
    ExtendNext,
    /// Shift+k: the same, up the list.
    ExtendPrevious,
    /// Close the composer if one is open, otherwise close the reader.
    Back,
    /// Archive the open conversation.
    Archive,
    /// Move it to the trash.
    Trash,
    /// Mark it as spam.
    Spam,
    /// Star it, or unstar it if it is already starred.
    ToggleStar,
    /// Mark it read, or unread if it is already read.
    ToggleRead,
    /// Reply to its newest message.
    Reply,
    /// Reply to everyone on it.
    ReplyAll,
    /// Forward it, with no recipients chosen yet.
    Forward,
    /// Pin it, or unpin it if it is already pinned.
    TogglePin,
    /// Mute it, or unmute it if it is muted: every picked conversation, or the open one.
    ToggleMute,
    /// Start a message that answers nothing.
    ///
    /// The one shortcut here that does not act on the conversation under the cursor, which is
    /// why it is also the one that works with an empty mailbox.
    Compose,
}

/// The shortcut a key press means with the keys the window ships with, or `None` for a key that
/// is not one. The window asks its [`crate::ui::keymap::Keymap`] instead, which may hold the user's
/// own keys.
///
/// Keys are named as the DOM names them, so the caller does not have to invent a second
/// vocabulary for the same events.
pub fn shortcut(key: &str, typing: bool) -> Option<Shortcut> {
    crate::ui::keymap::Keymap::default().action(key, typing)
}

/// What ⌘ does on its own, when it is held (`ui::chord::command`).
///
/// ⌘N writes a message wherever the caret is and ⌘⌫ trashes while reading. Every other key is
/// the Mac's or the field's — ⌘C is copy, ⌘A is select all — so the caller does not also ask
/// the keymap, which does not own chords. Esc is not here: the caller leaves it to close.
pub fn command_shortcut(key: &str, typing: bool) -> Option<Shortcut> {
    match key {
        "n" | "N" => Some(Shortcut::Compose),
        "Delete" | "Backspace" if !typing => Some(Shortcut::Trash),
        _ => None,
    }
}

/// The key a press means with Shift held: "J" and "K" whether the keyboard reported the
/// shifted letter or, as a synthesised press may, the plain one beside a Shift; and Shift with
/// an arrow is the same as Shift with its letter. Every other key is itself.
pub fn shifted(key: &str) -> &str {
    match key {
        "j" | "ArrowDown" => "J",
        "k" | "ArrowUp" => "K",
        other => other,
    }
}

/// What the snooze menu offers, as `(what the button says, the phrase it means)`.
///
/// A subset of the vocabulary `mailo snooze` accepts, not all of it: `+2h` and a date are things
/// to type, not things to click, and a menu that listed every accepted phrase would be a
/// reference card rather than a choice. The phrases are the same strings the command line
/// parses, so the two cannot drift into meaning different times.
pub fn snooze_choices() -> &'static [(&'static str, &'static str)] {
    &[
        ("Later today", "later"),
        ("This evening", "tonight"),
        ("Tomorrow", "tomorrow"),
        ("This weekend", "weekend"),
        ("Next week", "monday"),
    ]
}

/// One row of the label menu: the name, which label it is, and whether this conversation has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelChoice {
    pub name: String,
    pub id: LabelId,
    pub membership: Membership,
}

impl LabelChoice {
    /// What clicking it should do — the opposite of where the conversation is now.
    pub fn toggled(&self) -> Membership {
        self.membership.flip()
    }
}

/// The label menu for one conversation.
///
/// `known` is the shell's own index, so this stays a pure function of what is on screen. Names
/// are not unique — `UNIQUE (account, name)` is per account, so "travel" on two accounts is two
/// labels — and both are listed rather than merged, because giving a conversation on one account
/// the other account's "travel" is not something this menu can do and not something it should imply.
///
/// Sorted by name, with the ones already on the conversation first: the common act is taking a
/// label off the thing you are looking at, and it should not be a search.
pub fn label_menu(known: &[(String, LabelId)], summary: &ThreadSummary) -> Vec<LabelChoice> {
    let mut out: Vec<LabelChoice> = known
        .iter()
        .map(|(name, id)| LabelChoice {
            name: name.clone(),
            id: *id,
            membership: if summary.labels.contains(id) {
                Membership::In
            } else {
                Membership::Out
            },
        })
        .collect();
    out.sort_by(|a, b| {
        let on = |c: &LabelChoice| c.membership == Membership::In;
        on(b)
            .cmp(&on(a))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// The operation a shortcut performs on this conversation, if it is one the conversation allows.
///
/// Resolved through [`hover_actions`] rather than by a second table, so the keyboard can reach
/// exactly what the row's own buttons offer and nothing else — no un-archiving something that
/// was never in the inbox, and no starring something that is already starred.
///
/// `None` for [`Shortcut::Reply`] and [`Shortcut::ReplyAll`], which open a composer rather than
/// performing an operation, and for the movement keys.
pub fn op_for_shortcut(shortcut: Shortcut, summary: &ThreadSummary) -> Option<OpKind> {
    op_for_selection(shortcut, std::slice::from_ref(summary))
}

/// The operation a shortcut performs on several picked conversations at once, if any of them
/// allows it. [`op_for_shortcut`] is this for one.
///
/// One operation for all of them, never a toggle each: star with a selection that is half
/// starred stars the rest, as unread-first and unstarred-first is what every mail list does, and
/// a second press then unstars them all. Which conversations it then reaches is [`offers`]' to
/// say, per conversation.
pub fn op_for_selection(shortcut: Shortcut, summaries: &[ThreadSummary]) -> Option<OpKind> {
    let wanted: &[OpKind] = match shortcut {
        Shortcut::Archive => &[OpKind::Archive],
        Shortcut::Trash => &[OpKind::Trash],
        Shortcut::Spam => &[OpKind::Spam],
        Shortcut::ToggleStar => &[OpKind::Star, OpKind::Unstar],
        Shortcut::ToggleRead => &[OpKind::MarkRead, OpKind::MarkUnread],
        Shortcut::Next
        | Shortcut::Previous
        | Shortcut::ExtendNext
        | Shortcut::ExtendPrevious
        | Shortcut::Back => &[],
        // Both open or carry rather than performing a payload-free operation. `TogglePin` needs
        // the clock as well as the state, so it goes through `pin_op` instead, and `ToggleMute`
        // the state of every picked conversation at once (`mute_for_all`). `Compose` is not
        // about this conversation at all — it is the one shortcut with no `summary` to consult.
        Shortcut::Reply
        | Shortcut::ReplyAll
        | Shortcut::Forward
        | Shortcut::TogglePin
        | Shortcut::ToggleMute
        | Shortcut::Compose => &[],
    };
    wanted
        .iter()
        .copied()
        .find(|kind| summaries.iter().any(|summary| offers(summary, *kind)))
}

/// Whether `kind` is something this conversation can have done to it now: what its row offers
/// ([`hover_actions`]), and Spam, which the row has no room for, for anything not already there.
pub fn offers(summary: &ThreadSummary, kind: OpKind) -> bool {
    match kind {
        OpKind::Spam => !summary.mailboxes.contains(MailboxRole::Spam),
        other => hover_actions(summary).contains(&other),
    }
}

/// The conversation `Next` or `Previous` moves to.
///
/// Does not wrap. A list that jumps from the bottom back to the top loses the user's place in a
/// way that is hard to notice and easy to act on — the next keystroke archives the wrong thing.
/// Nothing open means the end you are coming from: the first row going down, the last going up.
pub fn step(current: Option<ThreadId>, ids: &[ThreadId], forward: bool) -> Option<ThreadId> {
    if ids.is_empty() {
        return None;
    }
    let Some(here) = current.and_then(|id| ids.iter().position(|it| *it == id)) else {
        // Nothing open, or something that is no longer in the list — archived out from under
        // the selection, most often. Start from the end the movement comes from.
        return Some(if forward { ids[0] } else { ids[ids.len() - 1] });
    };
    let next = if forward {
        here.checked_add(1).filter(|i| *i < ids.len())
    } else {
        here.checked_sub(1)
    };
    Some(ids[next.unwrap_or(here)])
}

/// The `Op` a hover button performs, where it needs no payload.
///
/// `None` for the ones that open a composer instead — a reply is a draft, not an operation.
pub fn op_for(kind: OpKind) -> Option<Op> {
    match kind {
        OpKind::Archive => Some(Op::Archive),
        OpKind::Trash => Some(Op::Trash),
        OpKind::Restore => Some(Op::Restore),
        OpKind::Spam => Some(Op::Spam),
        OpKind::MarkRead => Some(Op::SetRead(ReadState::Read)),
        OpKind::MarkUnread => Some(Op::SetRead(ReadState::Unread)),
        OpKind::Star => Some(Op::SetStar(Star::Starred)),
        OpKind::Unstar => Some(Op::SetStar(Star::Unstarred)),
        // These need a label picked, a draft created, a date chosen, or the conversation's own
        // state to say which way they go. Delete forever needs the confirmation sheet, which
        // applies it itself: no row, key or bar performs it on a press (`crate::ui::bin`).
        OpKind::Destroy
        | OpKind::AddLabel
        | OpKind::RemoveLabel
        | OpKind::Snooze
        | OpKind::Pin
        | OpKind::Mute
        | OpKind::FollowUp
        | OpKind::Reply
        | OpKind::ReplyAll
        | OpKind::Forward => None,
    }
}

/// The message a reply to this thread should answer.
///
/// The newest, which is what "reply" means to everyone except the person who wrote the
/// threading code. Replying to the thread's *root* would quote a conversation's opening line
/// back at someone who has since sent four more, and would set `In-Reply-To` to a message the
/// recipient's client threads above everything they last read.
///
/// Ties break on id so a thread with two messages at the same timestamp — which a bulk import
/// produces easily — picks the same one every time rather than alternating between renders.
pub fn reply_target(messages: &[Message]) -> Option<&Message> {
    messages.iter().max_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
    })
}

/// What the list pane should render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    Threads(Query),
    /// The inbox: its query, under the conversations whose follow-up reminder came back with no
    /// reply, on the accounts `scope` narrows to ([`mail_core::follow_up::returned`]).
    Inbox {
        query: Query,
        scope: Option<Filter>,
    },
    /// The drafts table, which no `Query` can express.
    Drafts,
    /// The conversations waiting on a reply, on the accounts `scope` narrows to.
    Waiting {
        scope: Option<Filter>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use mail_domain::id::new_account_id;

    fn summary(tweak: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
        let mut s = ThreadSummary {
            id: ThreadId::generate(),
            account: new_account_id(),
            subject: "s".into(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "a@b.test".into(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            message_count: 1,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
            mute: Mute::Unmuted,
            follow_up: mail_domain::FollowUp::Inactive,
        };
        tweak(&mut s);
        s
    }

    /// Whether a thread matches, with no corpus — none of these filters reads one.
    fn fits(filter: &Filter, summary: &ThreadSummary, now: DateTime<Utc>) -> bool {
        filter.fit(&mail_domain::MatchCtx {
            summary,
            corpus: None,
            folders: &[],
            now,
        })
    }

    #[test]
    fn inbox_is_just_a_filter() {
        // The plan claims a place is a saved filter rather than anything special. Assert it —
        // including the clause that makes snoozing mean something, which is still a filter and
        // not a special case in the list.
        let shell = Shell::default();
        assert_eq!(
            shell.query(20).filter,
            Filter::And(vec![
                Filter::InMailbox(MailboxRole::Inbox),
                Filter::Not(Box::new(pending_snooze())),
            ])
        );
    }

    #[test]
    fn a_snoozed_conversation_is_away_from_the_inbox_until_its_hour() {
        // Against `Filter::fit`, which is the same predicate the SQL side agrees with by
        // proptest. A button that leaves the conversation in the list does nothing.
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap();
        let inbox = Shell::default().query(20).filter;

        let awake = summary(|_| {});
        let away =
            summary(|s| s.snooze = Snooze::Until(now + chrono::TimeDelta::try_hours(3).unwrap()));
        let due =
            summary(|s| s.snooze = Snooze::Until(now - chrono::TimeDelta::try_hours(1).unwrap()));

        assert!(
            fits(&inbox, &awake, now),
            "an ordinary thread is in the inbox"
        );
        assert!(
            !fits(&inbox, &away, now),
            "a snoozed thread is still listed"
        );
        assert!(
            fits(&inbox, &due, now),
            "a snooze that has passed did not bring it back"
        );
    }

    #[test]
    fn the_snoozed_place_holds_only_what_is_still_away() {
        // A due thread is back in the inbox; listing it here as well would make "snoozed" mean
        // two different things in two places.
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap();
        let snoozed = pending_snooze();
        let away =
            summary(|s| s.snooze = Snooze::Until(now + chrono::TimeDelta::try_hours(3).unwrap()));
        let due =
            summary(|s| s.snooze = Snooze::Until(now - chrono::TimeDelta::try_hours(1).unwrap()));

        assert!(fits(&snoozed, &away, now));
        assert!(!fits(&snoozed, &due, now));
        assert!(!fits(&snoozed, &summary(|_| {}), now));
    }

    #[test]
    fn the_sidebar_offers_somewhere_to_find_a_snoozed_conversation() {
        // Otherwise snoozing is a way to lose mail.
        let places = default_places();
        let snoozed = places
            .iter()
            .find(|p| p.name == "Snoozed")
            .expect("no Snoozed place");
        assert_eq!(snoozed.source, Source::Mail(pending_snooze()));
    }

    #[test]
    fn searching_replaces_the_place_rather_than_narrowing_it() {
        // Searching while in Archive must not hide a result that lives in the Inbox.
        let mut shell = Shell::default();
        shell.select(1);
        shell.search = "  lunch  ".to_owned();
        assert_eq!(
            shell.query(20).filter,
            Filter::Text(TextMatch::Contains("lunch".to_owned())),
            "a search is global, and the box is trimmed"
        );
    }

    #[test]
    fn an_empty_search_box_falls_back_to_the_place() {
        let mut shell = Shell::default();
        let sent = shell
            .places
            .iter()
            .position(|place| place.name == "Sent")
            .expect("there is a Sent place");
        shell.select(sent);
        shell.search = "   ".to_owned();
        assert_eq!(shell.query(20).filter, Filter::InMailbox(MailboxRole::Sent));
    }

    #[test]
    fn the_sidebar_has_one_selected_row() {
        // The place the list shows, or the Today tab the open thread came from: never both.
        let mut shell = Shell::default();
        let thread = ThreadId::generate();
        assert!(shell.place_selected(0));
        assert_eq!(shell.selected_tab(), None);

        shell.open(thread);
        assert!(
            shell.place_selected(0),
            "a thread from the list keeps the place"
        );
        assert_eq!(shell.selected_tab(), None);

        shell.open_from_today(thread);
        assert!(
            !shell.place_selected(0),
            "a tab and a place are both selected"
        );
        assert_eq!(shell.selected_tab(), Some(thread));

        shell.select(1);
        assert!(shell.place_selected(1), "choosing a place takes it back");
        assert_eq!(shell.selected_tab(), None);

        shell.open_from_today(thread);
        shell.close();
        assert!(shell.place_selected(1));
        assert_eq!(shell.selected_tab(), None);
    }

    #[test]
    fn consent_to_remote_images_does_not_survive_changing_what_is_shown() {
        // A remote image is a read receipt. Agreeing to load one sender's is not agreeing to
        // the next message's, so consent is revoked by opening anything else.
        let mut shell = Shell {
            show_remote_images: true,
            ..Shell::default()
        };
        shell.open(ThreadId::generate());
        assert!(!shell.show_remote_images);

        shell.show_remote_images = true;
        shell.select(1);
        assert!(!shell.show_remote_images);
        assert_eq!(shell.policy().remote_images, RemoteImages::Blocked);
        // What a conversation opens under is what the cache warms under.
        assert_eq!(shell.policy(), SanitizePolicy::FRAME);

        shell.open(ThreadId::generate());
        shell.show_remote_images = true;
        shell.close();
        assert!(shell.open.is_none(), "close left the reader open");
        assert!(!shell.show_remote_images, "close kept image consent");
    }

    #[test]
    fn hover_actions_fit_where_the_thread_is() {
        let inbox = hover_actions(&summary(|_| {}));
        assert!(inbox.contains(&OpKind::Archive), "{inbox:?}");
        assert!(!inbox.contains(&OpKind::Restore), "nothing to restore from");

        let archived = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Archive)
        }));
        assert!(archived.contains(&OpKind::Restore), "{archived:?}");
        assert!(
            !archived.contains(&OpKind::Archive),
            "offering to archive what is archived is noise"
        );

        let trashed = hover_actions(&summary(|s| {
            s.mailboxes = MailboxSet::only(MailboxRole::Trash)
        }));
        assert!(!trashed.contains(&OpKind::Trash), "{trashed:?}");
    }

    #[test]
    fn the_read_and_star_buttons_show_the_opposite_of_the_current_state() {
        let unread = hover_actions(&summary(|_| {}));
        assert!(unread.contains(&OpKind::MarkRead));
        let read = hover_actions(&summary(|s| s.read = ReadState::Read));
        assert!(read.contains(&OpKind::MarkUnread));
    }

    #[test]
    fn actions_needing_a_payload_have_no_bare_op() {
        // A reply is a draft, not an operation, so a hover button cannot perform one.
        for kind in [
            OpKind::Reply,
            OpKind::Forward,
            OpKind::AddLabel,
            OpKind::Snooze,
        ] {
            assert!(op_for(kind).is_none(), "{kind:?} should open something");
        }
        assert_eq!(op_for(OpKind::Archive), Some(Op::Archive));
    }
}

#[cfg(test)]
mod composer_tests {
    use super::*;
    use mail_domain::id::new_account_id;

    fn addr(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    fn draft_of(to: Vec<Address>) -> Draft {
        Draft {
            id: DraftId::generate(),
            account: new_account_id(),
            identity: IdentityId::generate(),
            to,
            cc: Vec::new(),
            bcc: vec![addr(None, "blind@example.test")],
            subject: "Re: lunch".to_owned(),
            in_reply_to: Some(MessageId::generate()),
            forward_of: None,
            text: "body".to_owned(),
            html: None,
            attachments: Vec::new(),
            receipt: ReceiptRequest::Unrequested,
            openpgp: OpenPgp::None,
            smime: mail_domain::Smime::None,
            state: SendState::Editing,
            updated: chrono::Utc::now(),
        }
    }

    #[test]
    fn editing_changes_only_what_the_composer_shows() {
        // The composer has no Bcc box, no identity picker and no attachment list. If it built a
        // Draft from scratch it would silently drop all three — and the blind recipient would
        // stop receiving the message because the user fixed a typo in the subject.
        let base = draft_of(vec![addr(None, "ada@example.test")]);
        let mut editing = Composing::of(&base);
        editing.subject = "Re: lunch, moved".to_owned();
        let now = chrono::Utc::now();

        let edited = editing.apply_to(&base, now).unwrap();
        assert_eq!(edited.subject, "Re: lunch, moved");
        assert_eq!(edited.bcc, base.bcc, "the blind recipient was dropped");
        assert_eq!(edited.identity, base.identity);
        assert_eq!(edited.in_reply_to, base.in_reply_to, "threading was lost");
        assert_eq!(edited.id, base.id, "editing must not mint a second draft");
        assert_eq!(edited.updated, now);
    }

    #[test]
    fn a_bad_recipient_names_which_box_it_is_in() {
        let base = draft_of(Vec::new());
        let mut editing = Composing::of(&base);
        editing.to = "fine@example.test".to_owned();
        editing.cc = "not-an-address".to_owned();

        let err = editing
            .apply_to(&base, chrono::Utc::now())
            .expect_err("the Cc box is wrong");
        assert!(err.starts_with("Cc:"), "{err}");
    }

    #[test]
    fn opening_the_composer_shows_what_the_draft_holds() {
        let base = draft_of(vec![addr(Some("Ada"), "ada@example.test")]);
        let editing = Composing::of(&base);
        assert_eq!(editing.to, "Ada <ada@example.test>");
        assert_eq!(editing.subject, "Re: lunch");
        assert_eq!(editing.body, "body");
        assert_eq!(editing.draft, base.id);
        assert_eq!(editing.notice, None);
    }

    #[test]
    fn a_half_written_reply_survives_reading_another_thread() {
        // Composing sits beside reading rather than replacing it. A mode enum here would throw
        // the draft's widgets away the moment the user clicked another conversation.
        let base = draft_of(vec![addr(None, "ada@example.test")]);
        let mut shell = Shell::default();
        shell.compose(&base);
        shell.composing.as_mut().unwrap().body = "half a sentence".to_owned();

        shell.open(ThreadId::generate());
        assert_eq!(
            shell.composing.as_ref().map(|c| c.body.as_str()),
            Some("half a sentence"),
            "opening a thread discarded the composer"
        );

        shell.close_composer();
        assert!(shell.composing.is_none());
    }
}

#[cfg(test)]
mod reply_target_tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_domain::id::new_account_id;

    fn message(n: i64) -> Message {
        Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: new_account_id(),
            key: MessageKey::Rfc(format!("m{n}@example.test")),
            date: Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap(),
            from: Address {
                name: None,
                email: format!("s{n}@example.test"),
            },
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: format!("subject {n}"),
            in_reply_to: None,
            references: Vec::new(),
            rfc_message_id: Some(format!("m{n}@example.test")),
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: Vec::new(),
            body: Body::Absent,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn a_reply_answers_the_newest_message_not_the_first() {
        // Replying to the root quotes a conversation's opening line back at someone who has
        // since sent four more, and threads the reply above everything they last read.
        let messages = vec![message(0), message(300), message(100)];
        let target = reply_target(&messages).expect("a thread has messages");
        assert_eq!(target.subject, "subject 300");
    }

    #[test]
    fn the_choice_is_stable_when_two_messages_share_a_timestamp() {
        // A bulk import produces these easily. An unstable pick would reply to a different
        // message on each render, which the user would see as the quote changing under them.
        let messages = vec![message(5), message(5), message(5)];
        let first = reply_target(&messages).unwrap().id;
        for _ in 0..8 {
            assert_eq!(reply_target(&messages).unwrap().id, first);
        }
    }

    #[test]
    fn an_empty_thread_has_nothing_to_reply_to() {
        // Reachable: the messages are loaded one by one and any of them can fail to read.
        assert!(reply_target(&[]).is_none());
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    fn drafts_index(shell: &Shell) -> usize {
        shell
            .places
            .iter()
            .position(|p| p.source == Source::Drafts)
            .expect("there is a Drafts place")
    }

    #[test]
    fn the_drafts_place_lists_drafts_and_not_an_empty_mailbox() {
        // The bug this replaces: Drafts was `Filter::InMailbox(MailboxRole::Drafts)`, and a
        // draft has no thread and no mailbox, so the pane showed nothing for ever and said
        // nothing about why.
        let mut shell = Shell::default();
        shell.select(drafts_index(&shell));
        assert_eq!(shell.listing(50), Listing::Drafts);
    }

    #[test]
    fn every_other_place_still_lists_threads() {
        let shell = Shell::default();
        for (index, place) in shell.places.iter().enumerate() {
            if place.source == Source::Drafts {
                continue;
            }
            let mut shell = Shell::default();
            shell.select(index);
            assert!(
                !matches!(shell.listing(50), Listing::Drafts),
                "{} stopped listing threads",
                place.name
            );
        }
    }

    #[test]
    fn searching_while_in_drafts_searches_mail() {
        // Search is global. Someone typing in the box is looking for a message, not filtering
        // the handful of drafts already on screen.
        let mut shell = Shell::default();
        shell.select(drafts_index(&shell));
        shell.search = "invoice".to_owned();

        match shell.listing(50) {
            Listing::Threads(query) => assert_eq!(
                query.filter,
                Filter::Text(TextMatch::Contains("invoice".to_owned()))
            ),
            other => panic!("a search in Drafts must still search mail: {other:?}"),
        }
    }

    #[test]
    fn the_inbox_and_the_waiting_place_list_follow_ups_and_a_search_does_not() {
        let mut shell = Shell::default();
        assert!(
            matches!(shell.listing(50), Listing::Inbox { .. }),
            "the inbox lists returned reminders on top"
        );
        let waiting = shell
            .places
            .iter()
            .position(|p| p.source == Source::Waiting)
            .expect("there is a Waiting place");
        shell.select(waiting);
        assert_eq!(shell.listing(50), Listing::Waiting { scope: None });
        shell.search = "invoice".to_owned();
        assert!(matches!(shell.listing(50), Listing::Threads(_)));
    }

    #[test]
    fn a_blank_search_box_goes_back_to_the_drafts_list() {
        // Whitespace only is a blank box, not a search for a space.
        let mut shell = Shell::default();
        shell.select(drafts_index(&shell));
        shell.search = "   ".to_owned();
        assert_eq!(shell.listing(50), Listing::Drafts);
    }

    #[test]
    fn the_page_limit_reaches_the_query() {
        // "Show more" works by asking for a bigger page, so a limit that did not travel would
        // make the button do nothing at all.
        let shell = Shell::default();
        match shell.listing(250) {
            Listing::Inbox { query, .. } => assert_eq!(query.page.limit, 250),
            other => panic!("the Inbox lists its own query: {other:?}"),
        }
    }
}

/// Why the list pane has nothing in it, which decides what it should say.
///
/// The pane said "Nothing here." in every case, including the one every new user starts in: no
/// account configured at all. A mail client that has never been told whose mail to fetch looks
/// exactly like a mailbox that happens to be empty, and the shell has no way to add an account
/// — that is a terminal command — so "nothing here" was the end of the road rather than a state
/// with a way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nothing {
    /// No account has been added yet.
    NoAccount,
    /// A search that matched nothing. Carries the words, because what was searched for is the
    /// thing most likely to be mistyped.
    NoMatch(String),
    /// A folder with no mail in it, which is ordinary.
    EmptyFolder,
}

/// Why the list is empty.
pub fn nothing_to_show(accounts: usize, search: &str) -> Nothing {
    let needle = search.trim();
    if accounts == 0 {
        // Checked first: with no account there is nothing to search, and "nothing matches" would
        // send the user looking for a typo instead of for the setup step they have not done.
        Nothing::NoAccount
    } else if !needle.is_empty() {
        Nothing::NoMatch(needle.to_owned())
    } else {
        Nothing::EmptyFolder
    }
}

impl Nothing {
    /// What to say.
    pub fn message(&self) -> String {
        match self {
            Nothing::NoAccount => "No account".to_owned(),
            Nothing::NoMatch(needle) => format!("No results for \u{201c}{needle}\u{201d}"),
            Nothing::EmptyFolder => "Empty".to_owned(),
        }
    }
}

/// What a click on Discard means, given what the composer is currently showing.
///
/// A value rather than a branch inside the button, for the same reason `op_for` and
/// `hover_actions` are: the decision is the part that can be wrong, and a decision that only
/// exists inside a closure attached to a DOM node cannot be tested without a DOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discarding {
    /// Ask first. Discard deletes the draft now, and the button sits beside Close.
    Confirm,
    /// Asked and meant.
    Delete(DraftId),
}

/// Decide what a click on Discard should do.
///
/// `None` when there is no composer open, which the button cannot reach but the caller should
/// not have to assume.
pub fn discard_click(composing: Option<&Composing>) -> Option<Discarding> {
    let composing = composing?;
    Some(if composing.confirming_discard {
        Discarding::Delete(composing.draft)
    } else {
        Discarding::Confirm
    })
}

#[cfg(test)]
mod badge_tests {
    use super::*;

    #[test]
    fn a_mail_place_counts_its_unread_threads() {
        let inbox = Source::Mail(Filter::InMailbox(MailboxRole::Inbox));
        match badge_filter(&inbox) {
            Some(Filter::And(clauses)) => {
                assert!(clauses.contains(&Filter::InMailbox(MailboxRole::Inbox)));
                assert!(
                    clauses.contains(&Filter::Read(ReadState::Unread)),
                    "the badge counted every thread, not the unread ones: {clauses:?}"
                );
            }
            other => panic!("expected a conjunction, got {other:?}"),
        }
    }

    #[test]
    fn drafts_has_no_unread_badge() {
        // "3 unread drafts" is not a thing: a draft did not arrive and nobody failed to read it.
        assert_eq!(badge_filter(&Source::Drafts), None);
    }

    #[test]
    fn every_mail_place_gets_one_including_archive() {
        // A filter rule can file an unread message straight into Archive. A badgeless Archive is
        // then a message the user never finds out about.
        for place in default_places() {
            match &place.source {
                Source::Mail(_) | Source::Saved(_) => assert!(
                    badge_filter(&place.source).is_some(),
                    "{} has no badge",
                    place.name
                ),
                // The user wrote the last word in each of them: nothing there is unread news.
                Source::Drafts | Source::Waiting => {
                    assert!(badge_filter(&place.source).is_none())
                }
            }
        }
    }
}

/// The keyboard, which the shell did not have.
#[cfg(test)]
mod keyboard {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_core::place::pin_op;
    use mail_domain::id::new_account_id;

    fn summary(read: ReadState, star: Star, mailbox: MailboxRole) -> ThreadSummary {
        ThreadSummary {
            id: ThreadId::generate(),
            account: new_account_id(),
            subject: "lunch".to_owned(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "ada@example.test".to_owned(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: Utc.with_ymd_and_hms(2026, 9, 22, 0, 0, 0).unwrap(),
            message_count: 1,
            read,
            star,
            mailboxes: MailboxSet::only(mailbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
            mute: Mute::Unmuted,
            follow_up: mail_domain::FollowUp::Inactive,
        }
    }

    #[test]
    fn a_letter_is_a_shortcut_while_reading_and_a_letter_while_writing() {
        // The bug this exists to prevent: typing "e" into a reply archiving the conversation
        // behind it.
        assert_eq!(shortcut("e", false), Some(Shortcut::Archive));
        assert_eq!(shortcut("e", true), None);
        for key in ["j", "k", "s", "u", "r", "a", "#", "ArrowDown", "ArrowUp"] {
            assert!(shortcut(key, false).is_some(), "{key} does nothing");
            assert_eq!(shortcut(key, true), None, "{key} fired while typing");
        }
    }

    #[test]
    fn command_held_is_the_macs_and_never_a_bare_letter() {
        // ⌘N writes a message from anywhere, ⌘⌫ trashes while reading, and ⌘C, ⌘A and the
        // rest are the field's and the Mac's: none of them archives, replies or forwards.
        assert_eq!(command_shortcut("n", false), Some(Shortcut::Compose));
        assert_eq!(command_shortcut("N", true), Some(Shortcut::Compose));
        assert_eq!(command_shortcut("Backspace", false), Some(Shortcut::Trash));
        assert_eq!(command_shortcut("Backspace", true), None);
        for key in ["c", "a", "e", "r", "f", "p", "s", "u", "j", "k"] {
            assert_eq!(command_shortcut(key, false), None, "⌘{key} is not ours");
        }
        assert_eq!(shortcut("Escape", true), Some(Shortcut::Back));
        assert_eq!(command_shortcut("Escape", true), None);
    }

    #[test]
    fn escape_works_from_inside_the_thing_it_closes() {
        // The one exception, and it has to be: closing what you are typing in is not something
        // you can be asked to reach for the mouse to do.
        assert_eq!(shortcut("Escape", true), Some(Shortcut::Back));
        assert_eq!(shortcut("Escape", false), Some(Shortcut::Back));
    }

    #[test]
    fn a_key_that_is_not_a_shortcut_is_left_alone() {
        for key in ["z", "F5", "Tab", "Shift", " ", "1"] {
            assert_eq!(shortcut(key, false), None, "{key} was swallowed");
        }
    }

    #[test]
    fn star_and_read_resolve_against_what_the_thread_already_is() {
        // Toggles, and resolved through `hover_actions` so the keyboard and the row's buttons
        // cannot disagree about what is possible.
        let unstarred = summary(ReadState::Unread, Star::Unstarred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleStar, &unstarred),
            Some(OpKind::Star)
        );
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleRead, &unstarred),
            Some(OpKind::MarkRead)
        );
        let starred = summary(ReadState::Read, Star::Starred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleStar, &starred),
            Some(OpKind::Unstar)
        );
        assert_eq!(
            op_for_shortcut(Shortcut::ToggleRead, &starred),
            Some(OpKind::MarkUnread)
        );
    }

    #[test]
    fn forward_is_offered_on_every_conversation() {
        // It does not depend on where the conversation is or what state it is in: a forward
        // carries the message. It was reachable from nowhere before this.
        for mailbox in [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Trash,
            MailboxRole::Sent,
        ] {
            let summary = summary(ReadState::Read, Star::Unstarred, mailbox);
            assert!(
                hover_actions(&summary).contains(&OpKind::Forward),
                "no Forward on a conversation in {mailbox:?}"
            );
        }
    }

    #[test]
    fn f_forwards_and_is_not_an_operation() {
        assert_eq!(shortcut("f", false), Some(Shortcut::Forward));
        assert_eq!(shortcut("f", true), None, "fired while typing");
        let inbox = summary(ReadState::Read, Star::Unstarred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::Forward, &inbox),
            None,
            "a forward opens a composer rather than performing an operation"
        );
    }

    #[test]
    fn p_pins_and_unpins_and_the_rank_is_the_clock() {
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap();
        assert_eq!(shortcut("p", false), Some(Shortcut::TogglePin));
        assert_eq!(shortcut("p", true), None, "fired while typing");

        let unpinned = summary(ReadState::Read, Star::Unstarred, MailboxRole::Inbox);
        assert_eq!(
            pin_op(&unpinned, now),
            Op::SetPin(Pin::Rank(now.timestamp()))
        );
        let mut pinned = unpinned.clone();
        pinned.pin = Pin::Rank(1);
        assert_eq!(pin_op(&pinned, now), Op::SetPin(Pin::Unpinned));
        // Not an operation `op_for` can produce: the payload comes from the state and the clock.
        assert_eq!(op_for(OpKind::Pin), None);
        assert_eq!(op_for_shortcut(Shortcut::TogglePin, &unpinned), None);
    }

    #[test]
    fn m_mutes_and_unmutes_by_what_the_conversations_are() {
        assert_eq!(shortcut("m", false), Some(Shortcut::ToggleMute));
        assert_eq!(shortcut("m", true), None, "fired while typing");

        let unmuted = summary(ReadState::Read, Star::Unstarred, MailboxRole::Inbox);
        let mut muted = unmuted.clone();
        muted.mute = Mute::Muted;
        assert_eq!(mute_op(&unmuted), Op::SetMute(Mute::Muted));
        assert_eq!(mute_op(&muted), Op::SetMute(Mute::Unmuted));
        // Not an operation `op_for` can produce: the direction comes from the state.
        assert_eq!(op_for(OpKind::Mute), None);
        assert_eq!(op_for_shortcut(Shortcut::ToggleMute, &unmuted), None);

        // A selection: one mute for all, unmute only when every one is muted.
        const CASES: &[(&[Mute], Mute, &str)] = &[
            (&[Mute::Unmuted], Mute::Muted, "Mute"),
            (&[Mute::Muted], Mute::Unmuted, "Unmute"),
            (&[Mute::Muted, Mute::Unmuted], Mute::Muted, "Mute"),
            (&[Mute::Muted, Mute::Muted], Mute::Unmuted, "Unmute"),
            (&[], Mute::Muted, "Mute"),
        ];
        for (states, wanted, said) in CASES {
            let picked: Vec<ThreadSummary> = states
                .iter()
                .map(|mute| ThreadSummary {
                    mute: *mute,
                    ..unmuted.clone()
                })
                .collect();
            assert_eq!(mute_for_all(&picked), *wanted, "{states:?}");
            assert_eq!(mute_label(&picked), *said, "{states:?}");
        }
    }

    #[test]
    fn mute_is_offered_on_every_conversation() {
        for mailbox in [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Trash,
            MailboxRole::Sent,
        ] {
            let summary = summary(ReadState::Read, Star::Unstarred, mailbox);
            assert!(
                offers(&summary, OpKind::Mute),
                "{mailbox:?}: a mute is about replies still to come"
            );
        }
    }

    #[test]
    fn pin_is_offered_on_every_conversation() {
        for mailbox in [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Trash,
            MailboxRole::Sent,
        ] {
            let summary = summary(ReadState::Read, Star::Unstarred, mailbox);
            assert!(
                hover_actions(&summary).contains(&OpKind::Pin),
                "{mailbox:?}"
            );
        }
    }

    #[test]
    fn a_shortcut_cannot_reach_what_the_row_would_not_offer() {
        // Archiving something that is not in the inbox. The buttons do not offer it, so neither
        // does the key — one table, not two.
        let archived = summary(ReadState::Read, Star::Unstarred, MailboxRole::Archive);
        assert_eq!(op_for_shortcut(Shortcut::Archive, &archived), None);
        let inbox = summary(ReadState::Read, Star::Unstarred, MailboxRole::Inbox);
        assert_eq!(
            op_for_shortcut(Shortcut::Archive, &inbox),
            Some(OpKind::Archive)
        );
        // And the ones that are not operations at all.
        for shortcut in [Shortcut::Next, Shortcut::Back, Shortcut::Reply] {
            assert_eq!(op_for_shortcut(shortcut, &inbox), None);
        }
    }

    #[test]
    fn moving_stops_at_the_ends_rather_than_wrapping() {
        // A list that jumps from the bottom back to the top loses the user's place in a way that
        // is hard to notice and easy to act on: the next keystroke archives the wrong thing.
        let ids: Vec<ThreadId> = (0..3).map(|_| ThreadId::generate()).collect();
        assert_eq!(step(Some(ids[0]), &ids, true), Some(ids[1]));
        assert_eq!(
            step(Some(ids[2]), &ids, true),
            Some(ids[2]),
            "wrapped forward"
        );
        assert_eq!(step(Some(ids[1]), &ids, false), Some(ids[0]));
        assert_eq!(
            step(Some(ids[0]), &ids, false),
            Some(ids[0]),
            "wrapped back"
        );
    }

    #[test]
    fn moving_with_nothing_open_starts_from_the_end_it_comes_from() {
        let ids: Vec<ThreadId> = (0..3).map(|_| ThreadId::generate()).collect();
        assert_eq!(step(None, &ids, true), Some(ids[0]));
        assert_eq!(step(None, &ids, false), Some(ids[2]));
        assert_eq!(
            step(None, &[], true),
            None,
            "an empty list has nowhere to go"
        );
    }

    #[test]
    fn a_selection_that_left_the_list_does_not_strand_the_keyboard() {
        // Archiving the open conversation removes it from an inbox listing while it is still
        // `Shell::open`. The next keystroke has to go somewhere rather than nowhere.
        let ids: Vec<ThreadId> = (0..2).map(|_| ThreadId::generate()).collect();
        let gone = ThreadId::generate();
        assert_eq!(step(Some(gone), &ids, true), Some(ids[0]));
        assert_eq!(step(Some(gone), &ids, false), Some(ids[1]));
    }

    #[test]
    fn shift_with_j_k_or_an_arrow_extends_and_other_keys_are_themselves() {
        const CASES: &[(&str, Option<Shortcut>)] = &[
            ("j", Some(Shortcut::ExtendNext)),
            ("J", Some(Shortcut::ExtendNext)),
            ("ArrowDown", Some(Shortcut::ExtendNext)),
            ("k", Some(Shortcut::ExtendPrevious)),
            ("ArrowUp", Some(Shortcut::ExtendPrevious)),
            ("#", Some(Shortcut::Trash)),
            ("!", Some(Shortcut::Spam)),
        ];
        for (key, want) in CASES {
            assert_eq!(shortcut(shifted(key), false), *want, "Shift+{key}");
        }
        // Unshifted, j still moves and opens.
        assert_eq!(shortcut("j", false), Some(Shortcut::Next));
    }

    #[test]
    fn one_operation_for_a_whole_selection_not_a_toggle_each() {
        use MailboxRole::*;
        let starred = summary(ReadState::Read, Star::Starred, Inbox);
        let plain = summary(ReadState::Unread, Star::Unstarred, Inbox);
        let archived = summary(ReadState::Read, Star::Unstarred, Archive);
        let spam = summary(ReadState::Read, Star::Unstarred, Spam);
        type Case = (Shortcut, &'static [usize], Option<OpKind>);
        const CASES: &[Case] = &[
            // Half starred: star the rest; all starred: unstar.
            (Shortcut::ToggleStar, &[0, 1], Some(OpKind::Star)),
            (Shortcut::ToggleStar, &[0], Some(OpKind::Unstar)),
            // Any unread: mark read; none: mark unread.
            (Shortcut::ToggleRead, &[0, 1], Some(OpKind::MarkRead)),
            (Shortcut::ToggleRead, &[0, 2], Some(OpKind::MarkUnread)),
            // Archive when anything is still in the inbox; nothing when nothing is.
            (Shortcut::Archive, &[1, 2], Some(OpKind::Archive)),
            (Shortcut::Archive, &[2], None),
            (Shortcut::Spam, &[1, 3], Some(OpKind::Spam)),
            (Shortcut::Spam, &[3], None),
            (Shortcut::Trash, &[], None),
        ];
        let all = [starred, plain, archived, spam];
        for (action, which, want) in CASES {
            let picked: Vec<ThreadSummary> = which.iter().map(|n| all[*n].clone()).collect();
            assert_eq!(
                op_for_selection(*action, &picked),
                *want,
                "{action:?} on {which:?}"
            );
        }
        // And each conversation is reached only where it allows it.
        assert!(offers(&all[1], OpKind::Archive));
        assert!(!offers(&all[2], OpKind::Archive));
        assert!(!offers(&all[3], OpKind::Spam));
    }

    #[test]
    fn a_selection_is_the_place_s_and_a_plain_click_replaces_it() {
        let ids: Vec<ThreadId> = (0..4).map(|_| ThreadId::generate()).collect();
        let mut shell = Shell::default();
        shell.click(ids[1], Click::Plain, &ids);
        assert_eq!(shell.acted_on(&ids), vec![ids[1]], "the open one alone");
        shell.click(ids[3], Click::Range, &ids);
        assert_eq!(shell.acted_on(&ids), ids[1..].to_vec());
        assert!(shell.is_selected(ids[2], &ids));
        assert!(!shell.is_selected(ids[0], &ids));
        // A press on a picked row means the selection; on another row, that row.
        assert_eq!(shell.with_selection(ids[2], &ids), ids[1..].to_vec());
        assert_eq!(shell.with_selection(ids[0], &ids), vec![ids[0]]);
        // Another place: nothing picked, whatever the new list holds.
        shell.select(1);
        assert!(!shell.picked.any(&ids));
        assert_eq!(shell.acted_on(&ids), Vec::<ThreadId>::new());
        // Picked again, then a plain click: that row, open, and nothing picked.
        shell.pick_all(&ids);
        assert_eq!(shell.acted_on(&ids), ids);
        shell.click(ids[0], Click::Plain, &ids);
        assert_eq!(shell.acted_on(&ids), vec![ids[0]]);
        assert!(!shell.picked.any(&ids));
        // Esc: the picks go, the reader stays.
        shell.extend(Toward::Next, &ids);
        assert!(shell.unpick(&ids));
        assert!(!shell.unpick(&ids), "nothing left to let go");
        assert_eq!(shell.open, Some(ids[0]));
    }
}

/// Why the list pane is empty, which it never said.
#[cfg(test)]
mod nothing_tests {
    use super::*;

    #[test]
    fn no_account_beats_every_other_explanation() {
        // The first thing anyone sees. With no account there is nothing to search, so "nothing
        // matches" would send a new user hunting for a typo instead of doing the setup step.
        assert_eq!(nothing_to_show(0, ""), Nothing::NoAccount);
        assert_eq!(nothing_to_show(0, "invoice"), Nothing::NoAccount);
    }

    #[test]
    fn a_search_that_matched_nothing_says_what_was_searched_for() {
        // The words are the thing most likely to be mistyped, so they go in the message.
        assert_eq!(
            nothing_to_show(1, "invoice"),
            Nothing::NoMatch("invoice".to_owned())
        );
        assert!(nothing_to_show(1, "invoice").message().contains("invoice"));
        assert_eq!(
            nothing_to_show(1, "  invoice  "),
            Nothing::NoMatch("invoice".to_owned()),
            "whitespace is not the search"
        );
    }

    #[test]
    fn an_empty_folder_is_ordinary_and_says_so_briefly() {
        assert_eq!(nothing_to_show(2, ""), Nothing::EmptyFolder);
        assert_eq!(nothing_to_show(2, "   "), Nothing::EmptyFolder);
        assert_eq!(nothing_to_show(2, "").message(), "Empty");
    }
}

#[cfg(test)]
mod discarding {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_domain::id::new_account_id;

    fn composing() -> Composing {
        Composing {
            draft: DraftId::generate(),
            from: new_account_id(),
            to: "ada@example.test".to_owned(),
            cc: String::new(),
            subject: "Re: lunch".to_owned(),
            body: "never mind".to_owned(),
            attachments: Vec::new(),
            notice: None,
            confirming_discard: false,
        }
    }

    #[test]
    fn the_first_click_asks_and_the_second_deletes() {
        // Discard now removes the draft rather than closing the pane, and it sits beside Close.
        // One click must not be enough.
        let mut open = composing();
        assert_eq!(discard_click(Some(&open)), Some(Discarding::Confirm));
        open.confirming_discard = true;
        assert_eq!(
            discard_click(Some(&open)),
            Some(Discarding::Delete(open.draft))
        );
    }

    #[test]
    fn a_closed_composer_has_nothing_to_discard() {
        assert_eq!(discard_click(None), None);
    }

    #[test]
    fn reopening_the_composer_asks_again() {
        // The flag lives on the composer, so closing and reopening resets it. A confirmation
        // that survives the pane being closed is a trap set for the next draft.
        let draft = Draft {
            id: DraftId::generate(),
            account: new_account_id(),
            identity: IdentityId::generate(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Re: lunch".to_owned(),
            in_reply_to: None,
            forward_of: None,
            text: String::new(),
            html: None,
            attachments: vec![],
            receipt: ReceiptRequest::Unrequested,
            openpgp: OpenPgp::None,
            smime: mail_domain::Smime::None,
            state: SendState::Editing,
            updated: Utc.with_ymd_and_hms(2026, 9, 22, 0, 0, 0).unwrap(),
        };
        let mut shell = Shell::default();
        shell.compose(&draft);
        if let Some(c) = shell.composing.as_mut() {
            c.confirming_discard = true;
        }
        shell.close_composer();
        shell.compose(&draft);
        assert_eq!(
            discard_click(shell.composing.as_ref()),
            Some(Discarding::Confirm)
        );
    }
}

/// Which palette.
#[cfg(test)]
mod appearance {
    use super::*;

    #[test]
    fn system_follows_the_desktop_and_light_or_dark_do_not() {
        // mailo used to leave `data-theme` off for System so a media query could decide.
        // quire resolves the scheme in Rust and always writes one, so the guarantee is now
        // about the resolution: System is the desktop's, Light and Dark are themselves.
        const CASES: &[(Theme, Scheme, Scheme)] = &[
            (Theme::System, Scheme::Dark, Scheme::Dark),
            (Theme::System, Scheme::Light, Scheme::Light),
            (Theme::Light, Scheme::Dark, Scheme::Light),
            (Theme::Dark, Scheme::Light, Scheme::Dark),
        ];
        for &(theme, desktop, expect) in CASES {
            let system = SystemPrefs {
                scheme: desktop,
                ..SystemPrefs::default()
            };
            let resolved = resolve(ds::prelude::Appearance::default(), theme, system);
            assert_eq!(
                resolved.scheme, expect,
                "{theme:?} on a {desktop:?} desktop"
            );
        }
    }

    #[test]
    fn a_stored_theme_word_parses_or_does_not() {
        const CASES: &[(&str, Option<Theme>)] = &[
            ("system", Some(Theme::System)),
            ("light", Some(Theme::Light)),
            ("dark", Some(Theme::Dark)),
            ("", None),
            ("sepia", None),
            ("Dark", None),
        ];
        for &(word, expect) in CASES {
            assert_eq!(Theme::parse(word), expect, "{word:?}");
        }
    }
}

#[cfg(test)]
mod viewing_tests {
    use super::{Shell, Viewing};
    use mail_domain::{MessageId, ThreadId};

    #[test]
    fn a_viewer_turns_only_within_the_pages_it_knows() {
        let message = MessageId::generate();
        let at = |page: u32, pages: Option<u32>| Viewing {
            page,
            pages,
            ..Viewing::of(message, 1)
        };
        let cases = [
            ("forward, count known", at(0, Some(3)), 1, 1),
            ("forward from the last", at(2, Some(3)), 1, 2),
            ("back from the first", at(0, Some(3)), -1, 0),
            ("back", at(2, Some(3)), -1, 1),
            ("forward before the count is known", at(0, None), 1, 0),
            ("back before the count is known", at(1, None), -1, 0),
            ("a count of none", at(0, Some(0)), 1, 0),
        ];
        for (case, viewing, by, page) in cases {
            assert_eq!(viewing.turned(by).page, page, "{case}");
        }
    }

    #[test]
    fn opening_or_closing_a_conversation_closes_the_viewer() {
        let viewing = Some(Viewing::of(MessageId::generate(), 0));
        let mut shell = Shell {
            viewing,
            ..Shell::default()
        };
        shell.open(ThreadId::generate());
        assert_eq!(shell.viewing, None, "open");
        shell.viewing = viewing;
        shell.close();
        assert_eq!(shell.viewing, None, "close");
        shell.viewing = viewing;
        shell.select(1);
        assert_eq!(shell.viewing, None, "select");
    }
}
