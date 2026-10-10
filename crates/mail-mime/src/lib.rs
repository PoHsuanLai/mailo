//! MIME parsing, building, and HTML sanitization. Pure: bytes in, domain values out.
//!
//! Its own crate because these are not protocol machines — they have no [`IoNeed`] and no
//! state — and because `mail-app` must be able to sanitize without depending on `mail-proto`.
//!
//! [`IoNeed`]: https://docs.rs/mail-proto

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
pub use error::{MboxError, MimeError, NotHttpsUrl, NotMailto, RecordError, UnsafeUrl};
pub use graph::{GraphBody, GraphDraft, GraphImportance, graph_draft};
pub use imip::{CalendarPart, CalendarReply, calendar_part, calendar_reply};
pub use inline::{INLINE_BUDGET, embed_inline, embeddable};
pub use mailto::MailtoUri;
pub use mdn::{
    Human, OriginalHeaders, ReceiptAsk, Reporting, ReturnPath, Words, receipt, receipt_asked,
};
pub use parse::{Parsed, ParsedPart, RemotePart, parse, parse_reconstructed};
pub use print::{Labels, Options, Pages, Remote, Sheet, print_with, remote_images};
pub use reconstruct::{decode_part, left_on_server, reconstruct, sections_for};
pub use sanitize::{RemoteImages, SafeHtml, SanitizePolicy, Styles, sanitize};
pub use script::{Script, script_of};
pub use stamp::{restamp, with_blind};
pub use unsubscribe::{
    HttpsUrl, ListHeaders, ListId, Mailto, ONE_CLICK, Unsubscribe, list_headers,
};
