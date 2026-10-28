//! MIME parsing, building, and HTML sanitization. Pure: bytes in, domain values out.
//!
//! Its own crate because these are not protocol machines — they have no [`IoNeed`] and no
//! state — and because `mail-app` must be able to sanitize without depending on `mail-proto`.
//!
//! [`IoNeed`]: https://docs.rs/mailo-proto

pub mod archive;
pub mod auth;
pub mod bimi;
pub mod block;
pub mod build;
mod charset;
pub mod error;
pub mod graph;
pub mod imip;
pub mod inline;
pub mod mailto;
pub mod mdn;
#[cfg(feature = "openpgp")]
pub mod openpgp;
pub mod parse;
pub mod print;
pub mod reconstruct;
pub mod sanitize;
pub mod script;
#[cfg(feature = "smime")]
pub mod smime;
pub mod stamp;
#[cfg(feature = "testing")]
pub mod testing;
pub mod unsubscribe;

pub use auth::{AuthResults, Check, Receiver, Verdict, authentication_results};
pub use block::{
    Action, Align, Block, Delsp, Dir, Document, Flowed, ImgSrc, Inlined, LINK_REL, LINK_TARGET,
    Limits, ListKind, ParaStyle, Reached, RunStyle, SafeUrl, Shape, Span, TextColor, TextSize,
    from_html, from_text, is_mapped, mapped_tags, unflow,
};
pub use build::{
    Disclosure, Posting, Resending, base64_len, build, draft_copy, draft_message_id, posting,
    resent,
};
pub use error::{MboxError, MimeError, NotHttpsUrl, NotMailto, RecordError, UnsafeUrl};
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
pub use print::{Labels, Options, Pages, Remote, Sheet, escape_html, print_with, remote_images};
pub use reconstruct::{decode_part, left_on_server, reconstruct, sections_for};
pub use sanitize::{RemoteImages, SafeHtml, SanitizePolicy, Styles, sanitize};
pub use script::{Script, script_of};
pub use stamp::{is_resent, restamp, with_blind};
pub use unsubscribe::{
    HttpsUrl, ListHeaders, ListId, Mailto, ONE_CLICK, Unsubscribe, list_headers,
};

/// The README's examples, compiled and run as doctests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
