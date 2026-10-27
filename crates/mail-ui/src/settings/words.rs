//! The words mailo's settings are stored as. Each is a closed `Word` enum, so its schema key
//! names every value (design/22 section 9.1) and the Settings app picks its control by them: a
//! pair one of whose words is `off` is a switch.

use ds::prelude::Word;
use serde::{Deserialize, Serialize};

/// `window.provider_marks`: a provider chip shows the provider's icon, or its letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum ProviderMarks {
    #[default]
    Icons,
    Letters,
}

/// `window.snippets`: whether a conversation's row shows a line of its latest message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum Snippets {
    #[default]
    On,
    Off,
}

/// `window.label_chips`: whether a conversation's row shows its labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum LabelChips {
    #[default]
    On,
    Off,
}

/// `window.group_by_date`: whether the list is grouped under Today, Yesterday, This week and
/// Older.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum GroupByDate {
    On,
    #[default]
    Off,
}

/// `notifications.new_mail`: whether the window and `mailo watch` say new mail on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum NewMail {
    #[default]
    On,
    Off,
}

/// `notifications.notify_for`: whose new mail is said, everyone's or only the VIPs'.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum NotifyFor {
    #[default]
    #[word(label = "All new mail")]
    All,
    #[word(label = "VIPs only")]
    Vips,
}

/// `notifications.places`: which mailboxes' new mail is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum NotifyPlaces {
    #[default]
    #[word(label = "Inbox only")]
    Inbox,
    #[word(label = "Every followed mailbox")]
    Followed,
}

/// `notifications.sound`: the sound a banner plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum NotifySound {
    #[default]
    Default,
    Ping,
    Glass,
    Pop,
    Submarine,
    Silent,
}

/// `notifications.badge`: what the Dock's number counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum BadgeCounts {
    #[default]
    #[word(label = "The Inbox")]
    Inbox,
    #[word(label = "Every followed mailbox")]
    Followed,
}

/// `compose.spelling`: whether the composer marks misspelt words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum Spelling {
    #[default]
    On,
    Off,
}

/// `compose.copy_myself`: whether a message written here starts with the address it leaves from
/// on its Cc or its Bcc, so a copy of what was sent comes back to the person, as Apple Mail's
/// "Automatically Cc/Bcc myself" does. Off by default: most servers keep a Sent copy already.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum CopyMyself {
    #[default]
    Off,
    Cc,
    Bcc,
}

/// `compose.signature_above_quote`: whether a reply's signature starts above the quoted original
/// rather than under it, at the end, where Apple Mail puts it unless asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum SignatureAboveQuote {
    On,
    #[default]
    Off,
}

impl SignatureAboveQuote {
    /// Where a reply's signature goes, as composing says it.
    pub fn placement(self) -> mail_client::signature::Placement {
        match self {
            SignatureAboveQuote::On => mail_client::signature::Placement::AboveQuote,
            SignatureAboveQuote::Off => mail_client::signature::Placement::BelowQuote,
        }
    }
}

/// `compose.undo_send`: how long a message waits in the outbox after Send, which is how long its
/// Undo is offered. `Off` queues it to leave at once, with no Undo to press.
///
/// Five seconds is the default, the grace the composer always had. Stored as the number of
/// seconds with its unit, so the file reads as what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum UndoSend {
    Off,
    #[default]
    #[serde(rename = "5s")]
    #[word(slug = "5s", label = "5 seconds")]
    Seconds5,
    #[serde(rename = "10s")]
    #[word(slug = "10s", label = "10 seconds")]
    Seconds10,
    #[serde(rename = "20s")]
    #[word(slug = "20s", label = "20 seconds")]
    Seconds20,
    #[serde(rename = "30s")]
    #[word(slug = "30s", label = "30 seconds")]
    Seconds30,
}

impl UndoSend {
    /// How long Undo has.
    pub fn grace(self) -> chrono::TimeDelta {
        chrono::TimeDelta::seconds(match self {
            UndoSend::Off => 0,
            UndoSend::Seconds5 => 5,
            UndoSend::Seconds10 => 10,
            UndoSend::Seconds20 => 20,
            UndoSend::Seconds30 => 30,
        })
    }
}

/// `compose.format`: what a new message is written as. `Rich` sends the formatting with a plain
/// copy beside it (`multipart/alternative`); `Plain` sends the plain text alone, `text/plain`
/// with no HTML part, for lists and people who ask for nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum MessageFormat {
    #[default]
    #[word(label = "Rich text")]
    Rich,
    #[word(label = "Plain text")]
    Plain,
}

/// `compose.reply_same_format`: whether a reply is written as the message it answers was, plain
/// to plain and rich to rich, whatever `compose.format` says. On by default: a plain-text list
/// is answered in plain text without anyone having to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum ReplySameFormat {
    #[default]
    On,
    Off,
}

/// `reading.brand_logos`: whether a sender's verified logo (BIMI) is looked up and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum BrandLogos {
    On,
    #[default]
    Off,
}

/// `reading.remote_images`: when a message's remote images load without asking.
///
/// A remote image is fetched from the sender's server as the message is read, which tells the
/// sender when it was read and from which network address. `Ask` is the default: nothing loads
/// until "Show images" is pressed for the conversation. `Trusted` loads a sender's images when
/// their address is in `reading.trusted_image_senders` and the message proves it came from them;
/// `Always` loads every message's. Junk is asked about whatever this says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum LoadRemoteImages {
    #[default]
    Ask,
    #[word(label = "From senders I trust")]
    Trusted,
    Always,
}

/// `reading.read_receipts`: whether a message that asks for a read receipt gets one.
///
/// `Never` is the default, and answers nothing: the request is left as it came, and no receipt
/// goes. `Ask` puts the question over the message that asks, in the reader, and sends or
/// declines as the person answers. `Always` queues one the first time a message that asks is
/// opened, through the same path as `mailo receipt`, and only where it is safe to: never for mail
/// in Junk, and never when the receipt would go somewhere other than the domain the message came
/// back from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum ReadReceipts {
    #[default]
    Never,
    #[word(label = "Ask each time")]
    Ask,
    Always,
}

/// `reading.prefer_plain_text`: whether a message that carries a plain-text body beside its HTML
/// is shown as the plain text. One with only either body shows that one, whatever this says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum PreferPlainText {
    On,
    #[default]
    Off,
}

/// `reading.mark_read`: when an unread conversation the reader shows is marked read.
///
/// `OnOpen` is the default, and what opening one always did. `AfterDelay` waits
/// `reading.mark_read_delay` while it stays shown, and moving on before then leaves it unread;
/// `Never` leaves it to Mark as Read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum MarkRead {
    #[default]
    #[word(label = "On Open")]
    OnOpen,
    #[word(label = "After a Delay")]
    AfterDelay,
    Never,
}

/// `reading.mark_read_delay`: how long a conversation is shown before it is marked read, under
/// [`MarkRead::AfterDelay`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum MarkReadDelay {
    #[default]
    #[word(label = "2 Seconds")]
    Two,
    #[word(label = "5 Seconds")]
    Five,
    #[word(label = "10 Seconds")]
    Ten,
}

impl MarkReadDelay {
    /// The wait itself.
    pub fn duration(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            MarkReadDelay::Two => 2,
            MarkReadDelay::Five => 5,
            MarkReadDelay::Ten => 10,
        })
    }
}

/// `reading.text_zoom`: how large a message's text is drawn, in percent of its own size. Set
/// with Bigger, Smaller and Actual Size (⌘+, ⌘− and ⌘0), a step at a time ([`TextZoom::stepped`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TextZoom(pub u16);

/// A step of the text's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomStep {
    Bigger,
    Smaller,
    ActualSize,
}

impl TextZoom {
    /// The sizes a step goes between, as a browser's: from half size to three times.
    pub const LADDER: [u16; 12] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 300];
    /// The least and the most the setting takes; a file's value past either is held at it.
    pub const LEAST: u16 = 50;
    pub const MOST: u16 = 300;

    /// The size after `step`: the next rung up or down the ladder (a size between rungs goes to
    /// the nearest one past it), held at either end, or 100 for Actual Size.
    pub fn stepped(self, step: ZoomStep) -> TextZoom {
        let at = self.0.clamp(Self::LEAST, Self::MOST);
        let rung = match step {
            ZoomStep::ActualSize => 100,
            ZoomStep::Bigger => Self::LADDER
                .into_iter()
                .find(|rung| *rung > at)
                .unwrap_or(Self::MOST),
            ZoomStep::Smaller => Self::LADDER
                .into_iter()
                .rev()
                .find(|rung| *rung < at)
                .unwrap_or(Self::LEAST),
        };
        TextZoom(rung)
    }

    /// The size as a factor of the text's own, held to the setting's range.
    pub fn factor(self) -> f32 {
        f32::from(self.0.clamp(Self::LEAST, Self::MOST)) / 100.0
    }
}

impl Default for TextZoom {
    fn default() -> Self {
        TextZoom(100)
    }
}

/// `search.server_automatically`: whether a search is asked of the server without the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum ServerSearch {
    On,
    #[default]
    Off,
}

/// `attachments.remove_unedited`: when the files mailo wrote to open an attachment in its app are
/// removed again, as long as nothing has changed them since.
///
/// Only those files: each is written to mailo's own folder for opened attachments and recorded
/// as it is written, and a recorded file whose size or modification time has moved since is the
/// person's work and is left. Nothing saved to Downloads or to a folder the person chose is ever
/// removed. `Never` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum RemoveUnedited {
    #[default]
    Never,
    WhenMailoQuits,
    AfterMessageIsDeleted,
}

impl From<PreferPlainText> for mail_client::message::Prefer {
    fn from(plain: PreferPlainText) -> Self {
        match plain {
            PreferPlainText::On => mail_client::message::Prefer::Plain,
            PreferPlainText::Off => mail_client::message::Prefer::Html,
        }
    }
}

impl From<NotifyPlaces> for mail_client::notify::Places {
    fn from(places: NotifyPlaces) -> Self {
        match places {
            NotifyPlaces::Inbox => mail_client::notify::Places::Inbox,
            NotifyPlaces::Followed => mail_client::notify::Places::Followed,
        }
    }
}

/// `junk.filter` and the junk filter's other switches: on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum JunkSwitch {
    #[default]
    On,
    Off,
}

/// `junk.treat`: what is done with mail the junk filter calls junk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum JunkTreat {
    #[default]
    #[word(label = "Move to Spam")]
    Move,
    #[word(label = "Mark as Junk")]
    Mark,
}

/// `junk.order`: whether the junk filter runs before the rules or after them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum JunkOrder {
    #[default]
    #[word(label = "After my rules")]
    After,
    #[word(label = "Before my rules")]
    Before,
}

impl From<JunkSwitch> for mail_client::junk::Heed {
    fn from(switch: JunkSwitch) -> Self {
        match switch {
            JunkSwitch::On => mail_client::junk::Heed::Yes,
            JunkSwitch::Off => mail_client::junk::Heed::No,
        }
    }
}

impl From<BadgeCounts> for mail_client::notify::Places {
    fn from(counts: BadgeCounts) -> Self {
        match counts {
            BadgeCounts::Inbox => mail_client::notify::Places::Inbox,
            BadgeCounts::Followed => mail_client::notify::Places::Followed,
        }
    }
}

impl From<NotifySound> for mail_client::notify::Sound {
    fn from(sound: NotifySound) -> Self {
        match sound {
            NotifySound::Default => mail_client::notify::Sound::Default,
            NotifySound::Ping => mail_client::notify::Sound::Ping,
            NotifySound::Glass => mail_client::notify::Sound::Glass,
            NotifySound::Pop => mail_client::notify::Sound::Pop,
            NotifySound::Submarine => mail_client::notify::Sound::Submarine,
            NotifySound::Silent => mail_client::notify::Sound::Silent,
        }
    }
}

impl From<NewMail> for mail_client::notify::Setting {
    fn from(new_mail: NewMail) -> Self {
        match new_mail {
            NewMail::On => mail_client::notify::Setting::On,
            NewMail::Off => mail_client::notify::Setting::Off,
        }
    }
}

impl From<mail_client::notify::Setting> for NewMail {
    fn from(setting: mail_client::notify::Setting) -> Self {
        match setting {
            mail_client::notify::Setting::On => NewMail::On,
            mail_client::notify::Setting::Off => NewMail::Off,
        }
    }
}

impl From<NotifyFor> for mail_client::notify::Whom {
    fn from(notify_for: NotifyFor) -> Self {
        match notify_for {
            NotifyFor::All => mail_client::notify::Whom::Everyone,
            NotifyFor::Vips => mail_client::notify::Whom::Vips,
        }
    }
}

impl From<BrandLogos> for mail_client::bimi::Setting {
    fn from(logos: BrandLogos) -> Self {
        match logos {
            BrandLogos::On => mail_client::bimi::Setting::On,
            BrandLogos::Off => mail_client::bimi::Setting::Off,
        }
    }
}

impl From<mail_client::bimi::Setting> for BrandLogos {
    fn from(setting: mail_client::bimi::Setting) -> Self {
        match setting {
            mail_client::bimi::Setting::On => BrandLogos::On,
            mail_client::bimi::Setting::Off => BrandLogos::Off,
        }
    }
}

impl From<ServerSearch> for mail_client::server_search::Automatic {
    fn from(search: ServerSearch) -> Self {
        match search {
            ServerSearch::On => mail_client::server_search::Automatic::On,
            ServerSearch::Off => mail_client::server_search::Automatic::Off,
        }
    }
}

impl From<mail_client::server_search::Automatic> for ServerSearch {
    fn from(automatic: mail_client::server_search::Automatic) -> Self {
        match automatic {
            mail_client::server_search::Automatic::On => ServerSearch::On,
            mail_client::server_search::Automatic::Off => ServerSearch::Off,
        }
    }
}

/// `sync.while_quit`: whether `mailo watch` is started at login, so mail is checked, and new mail
/// said, while mailo is not running (`crate::login_item`). Off until the person turns it on: a
/// program that starts itself at login is theirs to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum WhileQuit {
    On,
    #[default]
    Off,
}

/// `privacy.protect`: which way remote images are fetched, once they are allowed to load.
///
/// `Off` fetches each image from its own host, which sees the reader's network address. `Relay`
/// fetches it through the relay at `privacy.relay` (a Camo relay or a token relay, by
/// `privacy.relay_kind`), so the host sees the relay's. `Proxy` sends every image fetch through
/// the proxy at `privacy.proxy` (Tor, a VPN's). Which images load at all is
/// `reading.remote_images`'s to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum Protect {
    #[default]
    Off,
    Relay,
    Proxy,
}

/// `privacy.relay_kind`: the URL format the relay at `privacy.relay` speaks.
///
/// `Camo` is GitHub's camo format, which go-camo and imgproxy serve: the image's URL signed with
/// HMAC-SHA1 under the relay's key. `Token` is `GET <relay>/i?u=<url>` with the key sent as a
/// bearer token, as `relay/image/worker.js` serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum RelayKind {
    #[default]
    Camo,
    Token,
}

/// `privacy.block_remote_content`: whether a message's remote content can be loaded at all.
/// `On` hides "Show images" and loads nothing remote, whatever `reading.remote_images` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum BlockRemote {
    #[default]
    Off,
    On,
}

/// `mailboxes.delete`: what Delete does to mail outside Trash and Spam.
///
/// Either way it moves to Trash first, with its Undo. `Now` then erases it from there as soon as
/// the server holds it in Trash (`mail_core::erase`), rather than when Trash's own time runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum DeleteMeans {
    #[default]
    #[word(label = "Moves to Trash")]
    Trash,
    #[word(label = "Deletes now")]
    Now,
}

/// `mailboxes.erase_trash` and `mailboxes.erase_spam`: how long a message stays in Trash or Spam
/// before it is deleted forever, counted from when it got there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum EraseAfter {
    #[default]
    Never,
    #[word(label = "After a day")]
    Day,
    #[word(label = "After a week")]
    Week,
    #[word(label = "After a month")]
    Month,
}

impl From<EraseAfter> for mail_client::erase::Keep {
    fn from(after: EraseAfter) -> Self {
        match after {
            EraseAfter::Never => mail_client::erase::Keep::Always,
            EraseAfter::Day => mail_client::erase::Keep::Day,
            EraseAfter::Week => mail_client::erase::Keep::Week,
            EraseAfter::Month => mail_client::erase::Keep::Month,
        }
    }
}

impl From<DeleteMeans> for mail_client::erase::Delete {
    fn from(means: DeleteMeans) -> Self {
        match means {
            DeleteMeans::Trash => mail_client::erase::Delete::ToTrash,
            DeleteMeans::Now => mail_client::erase::Delete::Now,
        }
    }
}
