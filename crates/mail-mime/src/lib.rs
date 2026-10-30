//! MIME parsing and building. Pure: bytes in, domain values out.
//!
//! Its own crate because these are not protocol machines — they have no [`IoNeed`] and no
//! state — and because `mail-app` must be able to read and build messages without depending on `mail-proto`.
//! What a parsed message shows (sanitizing, blocks, the printed page) is `mail-reader`'s.
//!
//! [`IoNeed`]: https://docs.rs/mailo-proto

pub mod auth;
pub mod bimi;
pub mod build;
mod charset;
pub mod error;
mod flowed;
pub mod graph;
pub mod imip;
pub mod inline;
pub mod mailto;
pub mod mdn;
pub mod parse;
pub mod reconstruct;
pub mod stamp;
pub mod unsubscribe;

pub use auth::{AuthResults, Check, Receiver, Verdict, authentication_results};
pub use build::{
    Disclosure, Posting, Resending, base64_len, build, draft_copy, draft_message_id, posting,
    resent,
};
pub use error::{MimeError, NotHttpsUrl, NotMailto, RecordError, UnsafeUrl};
pub use flowed::{Delsp, Flowed, unflow};
pub use graph::{GraphBody, GraphDraft, GraphImportance, graph_draft};
pub use imip::{CalendarPart, CalendarReply, calendar_part, calendar_reply};
pub use inline::{
    EmbeddedImage, INLINE_BUDGET, drawn_from, embed_inline, embed_inline_listed, embeddable,
};
pub use mailto::MailtoUri;
pub use mdn::{
    Human, OriginalHeaders, ReceiptAsk, Reporting, ReturnPath, Words, receipt, receipt_asked,
};
pub use parse::{Parsed, ParsedPart, RemotePart, parse, parse_reconstructed};
pub use reconstruct::{decode_part, left_on_server, reconstruct, sections_for};
pub use stamp::{is_resent, restamp, with_blind};
pub use unsubscribe::{
    HttpsUrl, ListHeaders, ListId, Mailto, ONE_CLICK, Unsubscribe, list_headers,
};

/// The README's examples, compiled and run as doctests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
