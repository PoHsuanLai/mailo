//! What a draft asks of a protection scheme when it is sent.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What OpenPGP, or S/MIME, is asked to do to a message when it is sent: the same three answers
/// for either scheme. A draft keeps one per scheme in its fields `openpgp` and `smime`, as an
/// `Option<Mode>`: `None` is "not asked", and is written as `"none"` ([`asked`]).
///
/// A draft asks one scheme, never both; sending refuses a draft that asks both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Signed: readable by anyone, provably from the sender (OpenPGP: RFC 3156 section 5;
    /// S/MIME: `multipart/signed`, RFC 8551 section 3.5.3).
    Sign,
    /// Encrypted to every recipient and the sender (OpenPGP: RFC 3156 section 4; S/MIME:
    /// enveloped-data, RFC 8551 section 3.3).
    Encrypt,
    /// Both: signed, then encrypted (OpenPGP: RFC 3156 section 6.2; S/MIME: RFC 8551 section 3.6).
    SignAndEncrypt,
}

impl Mode {
    /// Whether the message is signed.
    pub fn signs(self) -> bool {
        matches!(self, Mode::Sign | Mode::SignAndEncrypt)
    }

    /// Whether the message is encrypted.
    pub fn encrypts(self) -> bool {
        matches!(self, Mode::Encrypt | Mode::SignAndEncrypt)
    }
}

/// The serde form of a draft's `Option<Mode>`: the mode's name, or `"none"` for not asked. This
/// is how drafts have always been written, from before a mode could be absent.
///
/// For `#[serde(default, with = "mail_domain::protect::asked")]`.
pub mod asked {
    use super::*;
    use serde::de::IntoDeserializer;

    pub fn serialize<S: Serializer>(
        asked: &Option<Mode>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match asked {
            None => serializer.serialize_str("none"),
            Some(mode) => mode.serialize(serializer),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Mode>, D::Error> {
        let name = String::deserialize(deserializer)?;
        if name == "none" {
            return Ok(None);
        }
        Mode::deserialize(name.as_str().into_deserializer()).map(Some)
    }
}
