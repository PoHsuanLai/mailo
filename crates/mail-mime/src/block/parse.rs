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
    lower_html(sanitized, parts, images, html::Unheld::Drop)
}

/// [`from_html`] for a printout: a `cid:` image that cannot be embedded leaves its
/// description in the text instead of nothing.
pub(crate) fn from_html_describing(
    sanitized: &SafeHtml,
    parts: &[ParsedPart],
    images: RemoteImages,
) -> Document {
    lower_html(sanitized, parts, images, html::Unheld::Describe)
}

fn lower_html(
    sanitized: &SafeHtml,
    parts: &[ParsedPart],
    images: RemoteImages,
    unheld: html::Unheld,
) -> Document {
    let built = html::walk(sanitized.as_str(), parts, images, unheld);
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
    lower_text(text, flowed, text::Lines::Join)
}

/// [`from_text`] for a printout: a fixed line ends where the sender ended it.
pub(crate) fn from_text_keeping_lines(text: &str, flowed: Flowed) -> Document {
    lower_text(text, flowed, text::Lines::Keep)
}

fn lower_text(text: &str, flowed: Flowed, lines: text::Lines) -> Document {
    let built = text::parse(text, flowed, lines);
    let _ = Signals::plain();
    Document::new(
        built.blocks,
        Shape::Letter,
        built.reached,
        None,
        heaviness::score(&Signals::plain()),
    )
}
