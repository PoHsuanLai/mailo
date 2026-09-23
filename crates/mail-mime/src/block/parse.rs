//! The two ways into a [`Document`](super::kind::Document).
//!
//! HTML is read from [`crate::sanitize`]'s output. Plain text is read from the
//! body with its `format` and `delsp` parameters, which [`crate::parse`] does
//! not store — the caller passes them in.

use super::heaviness::{self, Signals};
use super::html;
use super::kind::{Document, Flowed, Shape};
use super::machine;
use super::text;
use crate::parse::ParsedPart;
use crate::sanitize::{RemoteImages, SafeHtml};

/// Lower sanitized HTML to blocks.
///
/// `images` decides remote images. It is not implied by how `sanitized` was
/// produced: a body sanitized with remote images allowed still carries the
/// URL, and passing [`RemoteImages::Blocked`] here records the host and drops
/// the URL. A body sanitized with images already blocked has no URL left to
/// name a host from.
pub fn from_html(sanitized: &SafeHtml, parts: &[ParsedPart], images: RemoteImages) -> Document {
    let built = html::walk(sanitized.as_str(), parts, images);
    let mut blocks = built.blocks;
    let heavy = heaviness::score(&built.signals);
    let (shape, primary) = machine::finish(&mut blocks, heaviness::is_heavy(&built.signals));
    Document::new(blocks, shape, built.reached, primary, heavy)
}

/// Lower plain text to blocks.
///
/// Heaviness is zero. There is no markup to score, so the shape is
/// [`Shape::Letter`] however the prose is arranged.
pub fn from_text(text: &str, flowed: Flowed) -> Document {
    let built = text::parse(text, flowed);
    let _ = Signals::plain();
    Document::new(
        built.blocks,
        Shape::Letter,
        built.reached,
        None,
        heaviness::score(&Signals::plain()),
    )
}
