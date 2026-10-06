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
