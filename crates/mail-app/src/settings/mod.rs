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
mod words;

pub use words::{
    BlockRemote, BrandLogos, CopyMyself, DeleteMeans, EraseAfter, GroupByDate, LabelChips,
    LoadRemoteImages, MarkRead, MarkReadDelay, MessageFormat, NewMail, NotifyFor, PreferPlainText,
    Protect, ProviderMarks, ReadReceipts, RelayKind, RemoveUnedited, ReplySameFormat, ServerSearch,
    SignatureAboveQuote, Snippets, Spelling, TextZoom, UndoSend, WhileQuit, ZoomStep,
};

use ds::prelude::Word;
use ds_settings::schema::{AppId, FilePath, KeySpec, Page, Schema, SettingsSchema, WordLabels};
use ds_settings::{AppName, ConfigRoot, FileName, Format, SettingsDoc, Store};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
        label = "Remove unedited downloads",
        help = "Remove the copies made to open attachments, unless you changed them.",
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
        label = "Block all remote content",
        help = "Never load a message's images from the web, and don't offer to.",
        section = "Privacy"
    )]
    pub block_remote_content: BlockRemote,
    #[settings(
        label = "Protect mail activity",
        help = "Hide your network address from the senders whose images load, through a relay or a proxy.",
        section = "Privacy"
    )]
    pub protect: Protect,
    #[settings(
        label = "Relay format",
        help = "Camo for go-camo or imgproxy; token for relay/image/worker.js.",
        section = "Privacy"
    )]
    pub relay_kind: RelayKind,
    #[settings(
        label = "Relay",
        help = "The relay's address, such as https://camo.example.com.",
        section = "Privacy"
    )]
    pub relay: String,
    #[settings(
        label = "Relay key",
        help = "The key the relay was set up with, or its token.",
        section = "Privacy"
    )]
    pub relay_key: String,
    #[settings(
        label = "Proxy",
        help = "An HTTP or SOCKS5 proxy, such as socks5h://127.0.0.1:9050 for Tor.",
        section = "Privacy"
    )]
    pub proxy: String,
}

impl PrivacySettings {
    /// The way remote images are fetched under these settings, or why it cannot be used: a
    /// relay or a proxy chosen with no usable address. An address that cannot be used is never
    /// quietly taken as `Off`: the fetch would go straight to the image's host, which is what
    /// the person chose against.
    pub fn route(&self) -> Result<mail_core::remote_image::Route, RouteProblem> {
        use mail_core::remote_image::Route;
        match self.protect {
            Protect::Off => Ok(Route::Direct),
            Protect::Relay => {
                // Only over `https`: the relay is sent the key or token, and every image's address.
                let base = url_with_host(&self.relay)
                    .filter(|url| url.scheme() == "https")
                    .ok_or(RouteProblem::Relay)?;
                let key = self.relay_key.trim().to_owned();
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
                let known = ["http://", "https://", "socks5://", "socks5h://"];
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
        help = "Delete mail forever once it has been in Trash this long.",
        section = "Mailboxes"
    )]
    pub erase_trash: EraseAfter,
    #[settings(
        label = "Erase Spam",
        help = "Delete mail forever once it has been in Spam this long.",
        section = "Mailboxes"
    )]
    pub erase_spam: EraseAfter,
}

impl MailboxSettings {
    /// What erasing Trash and Spam does under these settings.
    pub fn erasing(&self) -> mail_core::erase::Policy {
        mail_core::erase::Policy {
            delete: self.delete.into(),
            trash: self.erase_trash.into(),
            spam: self.erase_spam.into(),
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
    /// The proxy's address is not an `http`, `https` or `socks5` URL.
    Proxy,
}

/// `text` as a URL that names a host.
fn url_with_host(text: &str) -> Option<url::Url> {
    // What does not parse is no address.
    let url = url::Url::parse(text.trim()).ok()?;
    url.host_str().is_some().then_some(url)
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
        _ => {}
    }
    key
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

/// The person's own config root: the one `mail_core::config::config_dir()` is mailo's
/// directory in, as the window's is (`~/.config` on Linux, Application Support on macOS, the
/// roaming folder on Windows), so the command line, `watch` and every window read and write
/// one `settings.toml`. `None` where the machine has no config directory.
pub fn person_root() -> Option<ConfigRoot> {
    mail_core::config::config_dir().map(|dir| root_for(&dir))
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
