//! MIME parsing, building, and HTML sanitization. Pure: bytes in, domain values out.
//!
//! Its own crate because these are not protocol machines — they have no [`IoNeed`] and no
//! state — and because `mail-app` must be able to sanitize without depending on `mail-proto`.
//!
//! [`IoNeed`]: https://docs.rs/mail-proto

pub mod archive;
pub mod auth;
pub mod block;
pub mod build;
mod charset;
pub mod graph;
pub mod imip;
pub mod inline;
pub mod mailto;
pub mod mdn;
pub mod openpgp;
pub mod parse;
pub mod print;
pub mod reconstruct;
pub mod sanitize;
pub mod script;
pub mod smime;
pub mod stamp;
pub mod unsubscribe;

pub use auth::{AuthResults, Check, Receiver, Verdict, authentication_results};
pub use block::{
    Action, Block, Dir, Document, Flowed, ImgSrc, Inlined, LINK_REL, LINK_TARGET, Limits, Reached,
    SafeUrl, Shape, Span, from_html, from_text, is_mapped, mapped_tags,
};
pub use build::{Disclosure, Posting, build, posting};
pub use graph::{GraphBody, GraphDraft, GraphImportance, graph_draft};
pub use imip::{CalendarPart, CalendarReply, calendar_part, calendar_reply};
pub use inline::{INLINE_BUDGET, embed_inline, embeddable};
pub use mailto::MailtoUri;
pub use mdn::{OriginalHeaders, ReceiptAsk, Reporting, ReturnPath, receipt, receipt_asked};
pub use parse::{Parsed, ParsedPart, RemotePart, parse, parse_reconstructed};
pub use print::{Options, Pages, Remote, Sheet, print, print_with, remote_images};
pub use reconstruct::{decode_part, left_on_server, reconstruct, sections_for};
pub use sanitize::{RemoteImages, SafeHtml, SanitizePolicy, sanitize};
pub use script::{Script, script_of};
pub use stamp::restamp;
pub use unsubscribe::{
    HttpsUrl, ListHeaders, ListId, Mailto, ONE_CLICK, Unsubscribe, list_headers,
};

/// Something in the message, or in what we were asked to build, did not hold together.
///
/// Never a panic: every byte reaching this crate came off a network a stranger controls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MimeError {
    #[error("message could not be parsed: {0}")]
    Unparseable(String),
    #[error("required header missing: {0}")]
    MissingHeader(&'static str),
    #[error("attachment {0} was referenced but not supplied")]
    MissingPart(String),
    #[error("cannot build a message with no recipients")]
    NoRecipients,
    /// OpenPGP data that could not be read or made: a malformed key, a damaged message, a
    /// cryptographic failure. The text is the OpenPGP library's.
    #[error("OpenPGP: {0}")]
    OpenPgp(String),
    /// The secret key needs its passphrase, and none was given or the one given is wrong.
    #[error("the OpenPGP key {0} needs its passphrase")]
    KeyLocked(mail_domain::Fingerprint),
    /// Signing was asked for with no secret key to sign with.
    #[error("there is no OpenPGP key to sign with")]
    NoSigningKey,
    /// Encryption was asked for with no key to encrypt to.
    #[error("there is no OpenPGP key to encrypt to")]
    NoRecipientKeys,
    /// A recipient's key has no part that can be encrypted to (revoked, expired, sign-only).
    #[error("the OpenPGP key {0} cannot be encrypted to")]
    CannotEncryptTo(mail_domain::Fingerprint),
    /// S/MIME data that could not be read or made: a malformed certificate, message or PKCS#12
    /// file, an algorithm refused or not supported. The text says which.
    #[error("S/MIME: {0}")]
    Smime(String),
    /// A PKCS#12 file's password is wrong.
    #[error("that password does not open the PKCS#12 file")]
    WrongPassword,
}

impl mail_domain::Retryable for MimeError {
    fn retry(&self) -> mail_domain::Retry {
        // All of these are facts about the bytes or the keys, not about the network. Retrying re-runs the
        // same failure on the same input forever.
        mail_domain::Retry::Fatal(self.to_string())
    }
}
