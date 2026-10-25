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

/// `notifications.new_mail`: whether `mailo watch` says new mail on the desktop.
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
/// goes. `Always` queues one the first time a message that asks is opened, through the same path
/// as `mailo receipt`, and only where it is safe to: never for mail in Junk, and never when the
/// receipt would go somewhere other than the domain the message came back from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Word)]
#[serde(rename_all = "snake_case")]
#[word(case = snake)]
pub enum ReadReceipts {
    #[default]
    Never,
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

impl From<PreferPlainText> for mail_core::message::Prefer {
    fn from(plain: PreferPlainText) -> Self {
        match plain {
            PreferPlainText::On => mail_core::message::Prefer::Plain,
            PreferPlainText::Off => mail_core::message::Prefer::Html,
        }
    }
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
