//! mailo's own settings, the window-wide ones: one file, `mailo/settings.toml`, and the schema
//! that describes it (quire design/22 section 9).
//!
//! Each table is a struct deriving `SettingsSchema`, so the desktop's Settings app (detent) draws
//! a Mail page from the schema `mailo --write-schema <dir>` writes, and writes the same
//! file mailo reads; mailo's own Settings sheet draws the same keys. `ds_settings::Store` reads
//! the file leniently (a bad value costs only its key), writes it atomically and watches it, so a
//! change made in detent reaches an open window.
//!
//! What is not a key here stays where it was: the Spaces (`spaces.json`), the keyboard
//! (`keyboard.json`), which accounts are kept offline (`offline.json`, per account), and the
//! look, which is the desktop's (`quire/appearance.toml`).
//!
//! Before this file existed each switch had a file of its own. The first load with no
//! `settings.toml` reads them ([`legacy`]) and writes this file once; they are left where they
//! are and never written again.

mod legacy;
mod notices;
mod words;

pub use notices::{Notices, Shows};
pub use words::{
    BadgeCounts, BrandLogos, CopyMyself, DeleteMeans, EraseAfter, GroupByDate, JunkOrder,
    JunkSwitch, JunkTreat, KeepCalendar, LabelChips, LoadRemoteImages, MarkRead, MarkReadDelay,
    MessageFormat, NewMail, NotifyFor, NotifyPlaces, NotifySound, PreferPlainText, Protect,
    ProviderMarks, ReadReceipts, RelayKind, RemoveUnedited, ReplySameFormat, ServerSearch,
    SignatureAboveQuote, Snippets, Spelling, TextZoom, UndoSend, WhileQuit, ZoomStep,
};

use ds::prelude::Word;
use ds_settings::schema::{AppId, FilePath, KeySpec, Page, Schema, SettingsSchema, WordLabels};
use ds_settings::{AppName, ConfigRoot, FileName, Format, SettingsDoc, Store};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

/// The directory under the config root, and the schema's app id.
pub const APP: AppName = AppName("mailo");

/// The settings file, relative to the config root, as the schema names it.
const FILE: &str = "mailo/settings.toml";

/// The page mailo's keys are on in the Settings app: mailo's own.
fn page() -> Page {
    Page::App(APP.0.to_owned())
}

/// `window.*`: how the window draws mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "window", page = page())]
pub struct WindowSettings {
    #[settings(
        label = "Provider marks",
        help = "Show each account's provider as its logo or as a letter.",
        section = "Mail"
    )]
    pub provider_marks: ProviderMarks,
    #[settings(
        label = "Message preview",
        help = "Show a line of each conversation's latest message in the list.",
        section = "Message list"
    )]
    pub snippets: Snippets,
    #[settings(
        label = "Labels in the list",
        help = "Show each conversation's labels on its row.",
        section = "Message list"
    )]
    pub label_chips: LabelChips,
    #[settings(
        label = "Group by date",
        help = "Group the list under Today, Yesterday, This week and Older.",
        section = "Message list"
    )]
    pub group_by_date: GroupByDate,
    /// The places listed a message to a row, each by its `mail_core::conversation::place_key`:
    /// Organise by Conversation turned off in the list's ⋯ for each. Only the exceptions are
    /// kept, since a place lists conversations until told otherwise; a saved view keeps its own
    /// choice in the view. Not a key of the schema: a place's key means nothing in the Settings
    /// app, and the list is where it is changed.
    #[settings(skip)]
    pub single_messages: Vec<String>,
    /// The order each place's list was last sorted in from the list's ⋯, by the place's key
    /// (`mail_core::conversation::place_key`). A saved view keeps its order on the view instead. Not a
    /// key: nobody sets it from a Settings page, and the Settings app shows no map.
    #[settings(skip)]
    pub sorts: std::collections::BTreeMap<String, ListOrder>,
}

/// The places' orders as the window sorts by them, out of `window.sorts`.
pub fn sorts_of(window: &WindowSettings) -> std::collections::BTreeMap<String, mail_domain::Sort> {
    window
        .sorts
        .iter()
        .map(|(key, order)| (key.clone(), order.0))
        .collect()
}

/// A list's order as `settings.toml` holds it: the key's word and the way's, `"subject asc"`.
///
/// One string rather than a table of two: the file is read leniently, each value on its own, and
/// a table's two halves read alone are each missing the other, so a table would never read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListOrder(pub mail_domain::Sort);

impl Serialize for ListOrder {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let word = |value: serde_json::Result<serde_json::Value>| match value {
            Ok(serde_json::Value::String(word)) => Ok(word),
            _ => Err(serde::ser::Error::custom("a sort's words are strings")),
        };
        let property = word(serde_json::to_value(self.0.property))?;
        let dir = word(serde_json::to_value(self.0.dir))?;
        serializer.serialize_str(&format!("{property} {dir}"))
    }
}

impl<'de> Deserialize<'de> for ListOrder {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let refused = || serde::de::Error::custom(format!("not a list's order: {text:?}"));
        let (property, dir) = text.split_once(' ').ok_or_else(refused)?;
        let word = |word: &str| serde_json::Value::String(word.to_owned());
        Ok(ListOrder(mail_domain::Sort {
            property: serde_json::from_value(word(property)).map_err(|_| refused())?,
            dir: serde_json::from_value(word(dir)).map_err(|_| refused())?,
        }))
    }
}

/// `notifications.*`: what is said on the desktop.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "notifications", page = page())]
pub struct NotificationSettings {
    #[settings(
        label = "Notifications",
        help = "Show a notification when new mail arrives.",
        section = "Mail"
    )]
    pub new_mail: NewMail,
    #[settings(
        label = "Notify for",
        help = "Show notifications for all new mail, or only for mail from your VIPs.",
        section = "Mail"
    )]
    pub notify_for: NotifyFor,
    #[settings(
        label = "Notify in",
        help = "Which mailboxes announce new mail: the Inbox, or every mailbox the server lists as followed.",
        section = "Mail"
    )]
    pub places: NotifyPlaces,
    #[settings(
        label = "Sound",
        help = "The sound a notification plays.",
        section = "Mail"
    )]
    pub sound: NotifySound,
    #[settings(
        label = "Badge counts",
        help = "What the number on the app's icon counts: unread conversations in the Inbox, or in every followed mailbox.",
        section = "Mail"
    )]
    pub badge: BadgeCounts,
    /// The accounts that are never announced: addresses, in lowercase, switched off in
    /// Notifications. A key of its own, a list of text, so the desktop's Settings app shows it
    /// like any other.
    #[settings(
        label = "Muted accounts",
        help = "Accounts whose new mail is not announced.",
        section = "Mail"
    )]
    pub muted_accounts: Vec<String>,
}

impl NotificationSettings {
    /// What `mailo watch` and the window hear about and play, as the core takes it.
    pub fn interest(&self) -> mail_client::notify::Interest {
        mail_client::notify::Interest {
            places: self.places.into(),
            muted: self.muted_accounts.clone(),
            sound: self.sound.into(),
        }
    }
}

/// `compose.*`: writing mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "compose", page = page())]
pub struct ComposeSettings {
    #[settings(
        label = "Check spelling",
        help = "Underline misspelt words as you type.",
        section = "Composing"
    )]
    pub spelling: Spelling,
    #[settings(
        label = "Copy myself",
        help = "Put your own address on the Cc or Bcc of each message you write.",
        section = "Composing"
    )]
    pub copy_myself: CopyMyself,
    #[settings(
        label = "Signature above quoted text",
        help = "Put your signature above the message you reply to, rather than at the end.",
        section = "Composing"
    )]
    pub signature_above_quote: SignatureAboveQuote,
    #[settings(
        label = "Undo send",
        help = "How long a message waits after Send, so you can take it back.",
        section = "Composing"
    )]
    pub undo_send: UndoSend,
    #[settings(
        label = "Message format",
        help = "Plain text sends no formatting: the text alone, with no HTML part.",
        section = "Composing"
    )]
    pub format: MessageFormat,
    #[settings(
        label = "Reply in the same format as the original",
        help = "Answer plain text in plain text and rich text in rich text, whatever the message format says.",
        section = "Composing"
    )]
    pub reply_same_format: ReplySameFormat,
}

impl ComposeSettings {
    /// What a message is written as: `format`, except that a reply to a message whose format is
    /// known is written as that message was while `reply_same_format` is on. `original` is
    /// `None` for a message that answers nothing, and for one whose original's body is not here.
    pub fn format_for(&self, original: Option<MessageFormat>) -> MessageFormat {
        match (self.reply_same_format, original) {
            (ReplySameFormat::On, Some(theirs)) => theirs,
            _ => self.format,
        }
    }
}

/// `reading.*`: reading mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "reading", page = page())]
pub struct ReadingSettings {
    #[settings(
        label = "Brand logos",
        help = "Show a company's logo beside mail it has verified as its own.",
        section = "Reading"
    )]
    pub brand_logos: BrandLogos,
    /// Kept above the images' rows, so the senders whose images load stay under the choice
    /// they belong to.
    #[settings(
        label = "Send read receipts",
        help = "Tell the sender you opened a message that asks, except for junk or mail from another domain.",
        section = "Reading"
    )]
    pub read_receipts: ReadReceipts,
    #[settings(
        label = "Load remote images",
        help = "Images from the web can tell the sender when you open a message.",
        section = "Reading"
    )]
    pub remote_images: LoadRemoteImages,
    /// The senders whose remote images load under [`LoadRemoteImages::Trusted`]: addresses, in
    /// lowercase, added by "Always load from" in the reader. A key of its own, a list of text, so
    /// the desktop's Settings app can show and edit it like any other.
    #[settings(
        label = "Trusted senders",
        help = "Only for mail verified as theirs.",
        section = "Reading"
    )]
    pub trusted_image_senders: Vec<String>,
    #[settings(
        label = "Prefer plain text",
        help = "Show a message's plain-text version instead of its formatted one, when it has one.",
        section = "Reading"
    )]
    pub prefer_plain_text: PreferPlainText,
    #[settings(
        label = "Mark as read",
        help = "When a conversation you open stops being unread.",
        section = "Reading"
    )]
    pub mark_read: MarkRead,
    #[settings(
        label = "Mark as read after",
        help = "How long a conversation is shown before it is marked read, when that waits.",
        section = "Reading"
    )]
    pub mark_read_delay: MarkReadDelay,
    /// Set from the reader with Bigger, Smaller and Actual Size; the desktop's Settings app draws
    /// it as a slider.
    #[settings(
        label = "Message text size",
        help = "How large a message's text is drawn. Bigger and Smaller in the reader change it.",
        section = "Reading",
        range = "50..=300",
        unit = "%"
    )]
    pub text_zoom: TextZoom,
}

impl ReadingSettings {
    /// Whether `email` is one of [`Self::trusted_image_senders`], whatever its case: an address's
    /// domain has no case, and the list is kept in lowercase anyway.
    pub fn trusts_images_from(&self, email: &str) -> bool {
        let email = email.trim();
        self.trusted_image_senders
            .iter()
            .any(|trusted| trusted.eq_ignore_ascii_case(email))
    }

    /// "Always load from" `email`: the address joins the trusted senders, in lowercase and once.
    ///
    /// Under [`LoadRemoteImages::Ask`] that alone would change nothing anyone can see, since
    /// `Ask` never reads the list; so it also moves the setting to `Trusted`, which is what the
    /// person asked for in the only words the reader gave them. `Always` is left as it is: it
    /// already loads more than the list does.
    pub fn trust_images_from(&mut self, email: &str) {
        let email = email.trim().to_lowercase();
        if email.is_empty() {
            return;
        }
        if !self.trusts_images_from(&email) {
            self.trusted_image_senders.push(email);
        }
        if self.remote_images == LoadRemoteImages::Ask {
            self.remote_images = LoadRemoteImages::Trusted;
        }
    }
}

/// `search.*`: searching mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "search", page = page())]
pub struct SearchSettings {
    #[settings(
        label = "Search server",
        help = "Also search the mail on your servers as you type.",
        section = "Mail"
    )]
    pub server_automatically: ServerSearch,
}

/// `sync.*`: checking for mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "sync", page = page())]
pub struct SyncSettings {
    #[settings(
        label = "Check for mail when quit",
        help = "Keep checking for new mail after mailo quits, from each time you log in.",
        section = "Mail"
    )]
    pub while_quit: WhileQuit,
}

/// The schema path of [`SyncSettings::while_quit`], which the Settings sheet follows with the
/// login item.
pub const WHILE_QUIT: &str = "sync.while_quit";

/// `attachments.*`: where attachments are saved, and what becomes of the ones opened.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "attachments", page = page())]
pub struct AttachmentSettings {
    /// The folder Save writes into; empty is the system's Downloads folder
    /// (`mail_core::attach::downloads_dir`). A relative path is not a folder anyone can find, so
    /// it counts as empty too ([`AttachmentSettings::downloads_in`]).
    #[settings(
        label = "Save attachments to",
        help = "The folder Save puts an attachment in. Empty is your Downloads folder.",
        section = "Reading"
    )]
    pub downloads_folder: String,
    #[settings(
        label = "Remove opened attachments",
        help = "Copies made when you open an attachment are removed, unless you changed them. Saved files are never removed. Removal at quit happens when you quit normally.",
        section = "Reading"
    )]
    pub remove_unedited: RemoveUnedited,
}

impl AttachmentSettings {
    /// The folder Save writes into, when one was chosen: an absolute path, else `None`.
    pub fn downloads_in(&self) -> Option<std::path::PathBuf> {
        let folder = std::path::Path::new(self.downloads_folder.trim());
        folder.is_absolute().then(|| folder.to_path_buf())
    }
}

/// `privacy.*`: what a message's remote content may learn about the reader.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "privacy", page = page())]
pub struct PrivacySettings {
    #[settings(
        label = "Protect mail activity",
        help = "Hide your network address from the senders whose images load, through a relay or a proxy.",
        section = "Privacy"
    )]
    pub protect: Protect,
    #[settings(
        label = "Relay format",
        help = "Camo for go-camo or a relay with the same URL format; token for relay/image/worker.js.",
        section = "Privacy"
    )]
    pub relay_kind: RelayKind,
    #[settings(
        label = "Relay",
        help = "The relay's address, such as https://camo.example.com.",
        section = "Privacy"
    )]
    pub relay: String,
    /// How many times the relay's key was kept or forgotten: the one thing the file says of it.
    /// The key itself is in the keyring (`Net::set_relay_key`), never here, so a window that
    /// hears the count change reads it again.
    #[settings(skip)]
    pub relay_key_changed: u32,
    #[settings(
        label = "Proxy",
        help = "An HTTP or SOCKS5 proxy, such as socks5h://127.0.0.1:9050 for Tor. The proxy decides what it can reach.",
        section = "Privacy"
    )]
    pub proxy: String,
}

impl PrivacySettings {
    /// The way remote images are fetched under these settings, or why it cannot be used: a
    /// relay or a proxy chosen with no usable address. An address that cannot be used is never
    /// quietly taken as `Off`: the fetch would go straight to the image's host, which is what
    /// the person chose against.
    pub fn route(
        &self,
        relay_key: Option<&str>,
    ) -> Result<mail_client::remote_image::Route, RouteProblem> {
        use mail_client::remote_image::Route;
        match self.protect {
            Protect::Off => Ok(Route::Direct),
            Protect::Relay => {
                // Only over `https`: the relay is sent the key or token, and every image's address.
                let base = url_with_host(&self.relay)
                    .filter(|url| url.scheme() == "https")
                    .ok_or(RouteProblem::Relay)?;
                let key = relay_key.unwrap_or_default().trim().to_owned();
                if key.is_empty() {
                    return Err(RouteProblem::RelayKey);
                }
                Ok(match self.relay_kind {
                    RelayKind::Camo => Route::Camo { base, key },
                    RelayKind::Token => Route::Token { base, token: key },
                })
            }
            Protect::Proxy => {
                let url = self.proxy.trim();
                // `socks5://` is left out: it looks the image's host up here, outside the proxy, which is
                // what `socks5h://` does not.
                let known = ["http://", "https://", "socks5h://"];
                if !known.iter().any(|scheme| url.starts_with(scheme))
                    || url_with_host(url).is_none()
                {
                    return Err(RouteProblem::Proxy);
                }
                Ok(Route::Proxy {
                    url: url.to_owned(),
                })
            }
        }
    }
}

/// `mailboxes.*`: deleting, and how long Trash and Spam keep mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "mailboxes", page = page())]
pub struct MailboxSettings {
    #[settings(
        label = "Delete",
        help = "Move deleted mail to Trash, or delete it forever as soon as it gets there.",
        section = "Mailboxes"
    )]
    pub delete: DeleteMeans,
    #[settings(
        label = "Erase Trash",
        help = "Delete mail forever once it has been in Trash this long, while Mailo is open.",
        section = "Mailboxes"
    )]
    pub erase_trash: EraseAfter,
    #[settings(
        label = "Erase Spam",
        help = "Delete mail forever once it has been in Spam this long, while Mailo is open.",
        section = "Mailboxes"
    )]
    pub erase_spam: EraseAfter,
}

impl MailboxSettings {
    /// What erasing Trash and Spam does under these settings.
    pub fn erasing(&self) -> mail_client::erase::Policy {
        mail_client::erase::Policy {
            delete: self.delete.into(),
            trash: self.erase_trash.into(),
            spam: self.erase_spam.into(),
        }
    }
}

/// `junk.*`: the junk filter of mailo's own, which learns from Spam and Not Spam.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "junk", page = page())]
pub struct JunkSettings {
    #[settings(
        label = "Filter Junk Mail",
        help = "Judge new mail in the inbox by what you have marked as spam and not spam.",
        section = "Junk Mail"
    )]
    pub filter: JunkSwitch,
    #[settings(
        label = "When Mail Is Junk",
        help = "Move it to Spam, or leave it in the inbox with a Junk label.",
        section = "Junk Mail"
    )]
    pub treat: JunkTreat,
    #[settings(
        label = "Contacts",
        help = "Never call mail from someone in Contacts junk.",
        section = "Never Junk Mail From"
    )]
    pub spare_contacts: JunkSwitch,
    #[settings(
        label = "People You've Written To",
        help = "Never call mail from someone you have written to junk.",
        section = "Never Junk Mail From"
    )]
    pub spare_written_to: JunkSwitch,
    #[settings(
        label = "Mail Addressed to My Full Name",
        help = "Never call mail addressed to your full name junk.",
        section = "Never Junk Mail From"
    )]
    pub spare_full_name: JunkSwitch,
    #[settings(
        label = "Trust the Server's Spam Headers",
        help = "Take the server's own spam headers as its verdict.",
        section = "Junk Mail"
    )]
    pub trust_server: JunkSwitch,
    #[settings(
        label = "Filter Before My Rules",
        help = "Run the junk filter before your rules. Off runs it after them.",
        section = "Junk Mail"
    )]
    pub order: JunkOrder,
}

impl JunkSettings {
    /// How the junk filter behaves under these settings.
    pub fn policy(&self) -> mail_client::junk::Policy {
        mail_client::junk::Policy {
            filtering: match self.filter {
                JunkSwitch::On => mail_client::junk::Filtering::On,
                JunkSwitch::Off => mail_client::junk::Filtering::Off,
            },
            treat: match self.treat {
                JunkTreat::Move => mail_client::junk::Treat::Move,
                JunkTreat::Mark => mail_client::junk::Treat::Mark,
            },
            contacts: self.spare_contacts.into(),
            written_to: self.spare_written_to.into(),
            full_name: self.spare_full_name.into(),
            server_says: self.trust_server.into(),
            order: match self.order {
                JunkOrder::After => mail_client::junk::Order::AfterRules,
                JunkOrder::Before => mail_client::junk::Order::BeforeRules,
            },
        }
    }
}

/// Why the chosen way to fetch remote images cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteProblem {
    /// The relay's address is not an `https` URL.
    Relay,
    /// The relay has no key.
    RelayKey,
    /// The proxy's address is not an `http`, `https` or `socks5h` URL.
    Proxy,
}

/// `text` as a URL that names a host.
fn url_with_host(text: &str) -> Option<url::Url> {
    // What does not parse is no address.
    let url = url::Url::parse(text.trim()).ok()?;
    url.host_str().is_some().then_some(url)
}

/// `calendar.*`: the calendar invitations go to.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "calendar", page = page())]
pub struct CalendarSettings {
    #[settings(
        label = "Update my calendar",
        help = "When you answer an invitation, change the event if it is already in your calendar.",
        section = "Calendar"
    )]
    pub keep_in_step: KeepCalendar,
    /// The calendar "Add to Calendar" uses, by the id its store gives it. Empty is the store's
    /// first calendar that takes events; so is an id the store no longer has.
    #[settings(
        label = "Calendar for invitations",
        help = "Where Add to Calendar puts an event.",
        section = "Calendar"
    )]
    pub default: String,
}

/// `mailo/settings.toml`, whole.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MailSettings {
    pub window: WindowSettings,
    pub notifications: NotificationSettings,
    pub compose: ComposeSettings,
    pub reading: ReadingSettings,
    pub search: SearchSettings,
    pub sync: SyncSettings,
    pub attachments: AttachmentSettings,
    pub privacy: PrivacySettings,
    pub mailboxes: MailboxSettings,
    pub junk: JunkSettings,
    pub calendar: CalendarSettings,
}

impl SettingsDoc for MailSettings {
    const FILE: FileName = FileName("settings.toml");
    const FORMAT: Format = Format::Toml;
}

/// The one `mailo.settings.toml` the package installs: every table's keys under app `mailo`.
pub fn schema() -> Schema {
    let key = [
        WindowSettings::schema(),
        NotificationSettings::schema(),
        ComposeSettings::schema(),
        ReadingSettings::schema(),
        SearchSettings::schema(),
        SyncSettings::schema(),
        AttachmentSettings::schema(),
        PrivacySettings::schema(),
        MailboxSettings::schema(),
        JunkSettings::schema(),
        CalendarSettings::schema(),
    ]
    .into_iter()
    .flat_map(|schema| schema.key)
    .map(with_word_labels)
    .collect();
    Schema {
        app: AppId(APP.0.to_owned()),
        file: FilePath(FILE.to_owned()),
        version: 1,
        key,
        // mailo's settings structs declare no section footnotes.
        footnotes: Vec::new(),
        // mailo keeps no tables of another app's.
        foreign: Default::default(),
        // Every key sits on one of the built-in pages; mailo declares none of its own.
        pages: Vec::new(),
    }
}

/// `key` with the words a person reads for its values, where they are not the stored word
/// written out: "From senders I trust" for `trusted`. The derive leaves every key's labels empty,
/// and a Settings page shows a word without a label as itself.
fn with_word_labels(mut key: KeySpec) -> KeySpec {
    fn labels<T: Word>() -> WordLabels {
        WordLabels(
            T::ALL
                .iter()
                .map(|word| (word.slug().to_owned(), word.label().to_owned()))
                .collect(),
        )
    }
    match key.path.0.as_str() {
        "reading.remote_images" => key.labels = labels::<LoadRemoteImages>(),
        "compose.undo_send" => key.labels = labels::<UndoSend>(),
        "compose.format" => key.labels = labels::<MessageFormat>(),
        "reading.mark_read" => key.labels = labels::<MarkRead>(),
        "reading.mark_read_delay" => key.labels = labels::<MarkReadDelay>(),
        "notifications.notify_for" => key.labels = labels::<NotifyFor>(),
        "notifications.places" => key.labels = labels::<NotifyPlaces>(),
        "notifications.badge" => key.labels = labels::<BadgeCounts>(),
        "notifications.sound" => key.labels = labels::<NotifySound>(),
        _ => {}
    }
    key
}

/// What the number on the app's icon counts, as the person's settings say; the Inbox where there
/// are none to read. Asked on every poll of the count, so `settings.toml` is read again only
/// when the file has changed.
pub fn badge_places() -> mail_client::notify::Places {
    static MEMO: Mutex<Option<Counted>> = Mutex::new(None);
    person_root()
        .map(|root| badge_places_under(&root, &MEMO))
        // No config directory to read counts the Inbox.
        .unwrap_or_default()
}

/// What `settings.toml` looked like when the badge's places were last read from it, and what they
/// were.
type Counted = (
    Option<(std::time::SystemTime, u64)>,
    mail_client::notify::Places,
);

/// The places the badge counts under `root`, read from `memo` while the file is as it was.
fn badge_places_under(
    root: &ConfigRoot,
    memo: &Mutex<Option<Counted>>,
) -> mail_client::notify::Places {
    let looked = store(root.clone())
        .path::<MailSettings>()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
    let mut memo = memo.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((seen, places)) = *memo
        && seen == looked
    {
        return places;
    }
    let places: mail_client::notify::Places = load(root).notifications.badge.into();
    // A first read may have written the file (from the old per-switch ones): look again.
    let after = store(root.clone())
        .path::<MailSettings>()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
    *memo = Some((after, places));
    places
}

/// The config root a mailo config directory stands for: `~/.config/mailo` is `~/.config`, the
/// root it is mailo's directory in. Any other directory (a test's scratch, a dev script's) is a
/// root of its own, so `settings.toml` lands in its `mailo/` and never beside it.
pub fn root_for(config_dir: &Path) -> ConfigRoot {
    match (config_dir.file_name(), config_dir.parent()) {
        (Some(name), Some(parent)) if name == APP.0 => ConfigRoot::Scratch(parent.to_path_buf()),
        _ => ConfigRoot::Scratch(config_dir.to_path_buf()),
    }
}

/// The person's own config root: the one `mail_client::config::config_dir()` is mailo's
/// directory in, as the window's is (`~/.config` on Linux, Application Support on macOS, the
/// roaming folder on Windows), so the command line, `watch` and every window read and write
/// one `settings.toml`. `None` where the machine has no config directory.
pub fn person_root() -> Option<ConfigRoot> {
    mail_client::config::config_dir().map(|dir| root_for(&dir))
}

/// How the junk filter behaves for a sync this process starts: the person's settings, or the
/// defaults with no config directory.
pub fn junk_policy() -> mail_client::junk::Policy {
    person_root()
        .map(|root| load(&root))
        .unwrap_or_default()
        .junk
        .policy()
}

/// The store `settings.toml` lives in, under `root`.
pub fn store(root: ConfigRoot) -> Store {
    Store::new(root, APP)
}

/// The settings in force under `root`. With no `settings.toml` yet, what the old per-switch files
/// said, written once as `settings.toml`; with neither, the defaults. A file that cannot be read
/// or written is the defaults: these are preferences, and a window opens without them.
pub fn load(root: &ConfigRoot) -> MailSettings {
    let store = store(root.clone());
    if store
        .path::<MailSettings>()
        .is_some_and(|path| path.exists())
    {
        fold_block_remote(&store);
        return store.load::<MailSettings>().value;
    }
    let Some(dir) = store.dir() else {
        return MailSettings::default();
    };
    let carried = legacy::read(&dir);
    if carried != MailSettings::default() {
        let _ = store.save(&carried);
    }
    carried
}

/// Carry `privacy.block_remote_content`, the switch `reading.remote_images = "never"` replaced,
/// into the file: `"on"` becomes `Never`, and the old key goes either way, so it is not read
/// again over a later choice. A file that does not read is left alone.
fn fold_block_remote(store: &Store) {
    let Some(path) = store.path::<MailSettings>() else {
        return;
    };
    let path = path.to_path_buf();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut table) = text.parse::<toml::Table>() else {
        return;
    };
    let Some(old) = table
        .get_mut("privacy")
        .and_then(toml::Value::as_table_mut)
        .and_then(|privacy| privacy.remove("block_remote_content"))
    else {
        return;
    };
    if old.as_str() == Some("on") {
        let reading = table
            .entry("reading")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let Some(reading) = reading.as_table_mut() else {
            return;
        };
        reading.insert(
            "remote_images".to_owned(),
            toml::Value::String("never".to_owned()),
        );
    }
    let Ok(text) = toml::to_string(&table) else {
        return;
    };
    let staged = path.with_extension("toml.tmp");
    if let Err(e) = std::fs::write(&staged, text).and_then(|()| std::fs::rename(&staged, &path)) {
        log::warn!("settings.toml: the old remote content switch was not carried over: {e}");
    }
}

/// Whether what `settings.toml` under `root` says about `privacy.*` cannot be trusted: the whole
/// file does not read, or a `privacy` key holds a value the setting does not take. Then the
/// settings in force are the defaults, which would fetch images straight from their hosts
/// whatever the person chose, so images wait until the file is mended.
pub fn privacy_in_doubt(root: &ConfigRoot) -> bool {
    store(root.clone())
        .load::<MailSettings>()
        .invalid
        .iter()
        .any(|key| key.path.is_empty() || key.path.starts_with("privacy"))
}

/// Why `settings.toml` under `root` cannot be used as it is: the whole file, or each key whose
/// value is not one the setting takes, as "key: reason". `None` when it is missing or reads. A key
/// this build does not know is not damage: it is kept as it is.
pub fn damage(root: &ConfigRoot) -> Option<String> {
    let invalid = store(root.clone()).load::<MailSettings>().invalid;
    let said: Vec<String> = invalid
        .iter()
        .map(|key| match key.path.as_str() {
            "" => key.reason.clone(),
            path => format!("{path}: {}", key.reason),
        })
        .collect();
    (!said.is_empty()).then(|| said.join("; "))
}

/// Change the settings under `root` with `edit` and write them, returning what was written. A
/// `settings.toml` that cannot be used is left as it is, and nothing changes: writing would put
/// the defaults in place of what it held.
pub fn change(
    root: &ConfigRoot,
    edit: impl FnOnce(&mut MailSettings),
) -> Result<MailSettings, String> {
    if let Some(why) = damage(root) {
        return Err(why);
    }
    let mut settings = load(root);
    edit(&mut settings);
    store(root.clone())
        .save(&settings)
        .map_err(|e| e.to_string())?;
    Ok(settings)
}

/// `mailo --write-schema <dir>`: write `mailo.settings.toml` into `dir`.
pub fn write_schema(dir: &Path) -> Result<String, String> {
    let path = schema().write_to(dir).map_err(|e| e.to_string())?;
    Ok(format!("wrote {}\n", path.display()))
}

#[cfg(test)]
mod tests;
