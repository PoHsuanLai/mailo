//! How a received message becomes what is read. Pure: parsed parts in, markup and blocks out.
//!
//! Its own crate because none of it touches a store or a socket, and because what a message
//! shows is decided here once, not in every program that draws it: untrusted HTML made safe
//! ([`sanitize`]), lowered to blocks a reader can draw ([`block`]), a reply's quoted tail folded
//! ([`quote`]), the script a message is written in ([`script`]), and the printed page ([`print`]).
//! Parsing the bytes into parts is `mail-mime`'s; this starts from its output.

pub mod block;
pub mod frame;
pub mod print;
pub mod quote;
pub mod sanitize;
pub mod script;

pub use block::{
    Action, Align, Block, Dir, Document, ImgSrc, Inlined, LINK_REL, LINK_TARGET, Limits, ListKind,
    ParaStyle, Reached, RunStyle, SafeUrl, Shape, Span, TextColor, TextSize, from_html, from_text,
    is_mapped, mapped_tags,
};
pub use frame::{FOUND_CURRENT, FOUND_MARK, FrameBody, Prefer, Source, frame_of};
pub use print::{Labels, Options, Pages, Remote, Sheet, escape_html, print_with, remote_images};
pub use sanitize::{RemoteImages, SafeHtml, SanitizePolicy, Styles, sanitize};
pub use script::{Script, script_of, script_of_parsed};

/// The README's examples, compiled and run as doctests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
