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
    pub fn placement(self) -> mail_core::signature::Placement {
        match self {
            SignatureAboveQuote::On => mail_core::signature::Placement::AboveQuote,
            SignatureAboveQuote::Off => mail_core::signature::Placement::BelowQuote,
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

/// `search.server_automatically`: whether a search is asked of the server without the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum ServerSearch {
    On,
    #[default]
    Off,
}

impl From<NewMail> for mail_core::notify::Setting {
    fn from(new_mail: NewMail) -> Self {
        match new_mail {
            NewMail::On => mail_core::notify::Setting::On,
            NewMail::Off => mail_core::notify::Setting::Off,
        }
    }
}

impl From<mail_core::notify::Setting> for NewMail {
    fn from(setting: mail_core::notify::Setting) -> Self {
        match setting {
            mail_core::notify::Setting::On => NewMail::On,
            mail_core::notify::Setting::Off => NewMail::Off,
        }
    }
}

impl From<BrandLogos> for mail_core::bimi::Setting {
    fn from(logos: BrandLogos) -> Self {
        match logos {
            BrandLogos::On => mail_core::bimi::Setting::On,
            BrandLogos::Off => mail_core::bimi::Setting::Off,
        }
    }
}

impl From<mail_core::bimi::Setting> for BrandLogos {
    fn from(setting: mail_core::bimi::Setting) -> Self {
        match setting {
            mail_core::bimi::Setting::On => BrandLogos::On,
            mail_core::bimi::Setting::Off => BrandLogos::Off,
        }
    }
}

impl From<ServerSearch> for mail_core::server_search::Automatic {
    fn from(search: ServerSearch) -> Self {
        match search {
            ServerSearch::On => mail_core::server_search::Automatic::On,
            ServerSearch::Off => mail_core::server_search::Automatic::Off,
        }
    }
}

impl From<mail_core::server_search::Automatic> for ServerSearch {
    fn from(automatic: mail_core::server_search::Automatic) -> Self {
        match automatic {
            mail_core::server_search::Automatic::On => ServerSearch::On,
            mail_core::server_search::Automatic::Off => ServerSearch::Off,
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
