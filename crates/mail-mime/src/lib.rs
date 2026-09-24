//! MIME parsing, building, and HTML sanitization. Pure: bytes in, domain values out.
//!
//! Its own crate because these are not protocol machines — they have no [`IoNeed`] and no
//! state — and because `mail-app` must be able to sanitize without depending on `mail-proto`.
//!
//! [`IoNeed`]: https://docs.rs/mail-proto

pub mod archive;
pub mod block;
pub mod build;
mod charset;
pub mod graph;
pub mod imip;
pub mod inline;
pub mod mdn;
pub mod parse;
pub mod print;
pub mod reconstruct;
pub mod sanitize;
pub mod stamp;
pub mod unsubscribe;

pub use block::{
    Action, Block, Dir, Document, Flowed, ImgSrc, Inlined, LINK_REL, LINK_TARGET, Limits, Reached,
    SafeUrl, Shape, Span, from_html, from_text, is_mapped, mapped_tags,
};
pub use build::{Disclosure, Posting, build, posting};
pub use graph::{GraphBody, GraphDraft, GraphImportance, graph_draft};
pub use imip::{CalendarPart, CalendarReply, calendar_part, calendar_reply};
pub use inline::{INLINE_BUDGET, embed_inline, embeddable};
pub use mdn::{OriginalHeaders, ReceiptAsk, Reporting, ReturnPath, receipt, receipt_asked};
pub use parse::{Parsed, ParsedPart, RemotePart, parse, parse_reconstructed};
pub use print::{Pages, Sheet, print};
pub use reconstruct::{decode_part, reconstruct, sections_for};
pub use sanitize::{RemoteImages, SafeHtml, SanitizePolicy, sanitize};
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
}

impl mail_domain::Retryable for MimeError {
    fn retry(&self) -> mail_domain::Retry {
        // All four are facts about the bytes, not about the network. Retrying re-runs the
        // same failure on the same input forever.
        mail_domain::Retry::Fatal(self.to_string())
    }
}
