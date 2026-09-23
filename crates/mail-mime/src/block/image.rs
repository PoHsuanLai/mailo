//! `cid:` and remote images, resolved while the walk is reading the tag.
//!
//! An inline image becomes a `data:` URI only when [`crate::embeddable`]
//! accepts the part's declared type. The spelling written into the URI is the
//! allowlist's, never the message's. A remote image becomes a [`ImgSrc::Remote`]
//! when the reader has allowed them, and a [`ImgSrc::Blocked`] naming the host
//! when they have not. The blocked URL is not stored.

use super::kind::{ImgSrc, Inlined};
use super::url::SafeUrl;
use super::url::image_host;
use crate::inline::{INLINE_BUDGET, embeddable};
use crate::parse::ParsedPart;
use crate::sanitize::RemoteImages;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mail_domain::Inline;

/// Resolve an `img src` into the image the document will carry.
pub(crate) fn resolve(
    src: &str,
    parts: &[ParsedPart],
    images: RemoteImages,
    spent: &mut usize,
) -> Option<ImgSrc> {
    let src = src.trim();
    if let Some(reference) = src.strip_prefix("cid:") {
        return inline_part(reference, parts, spent).map(ImgSrc::Inline);
    }
    match images {
        RemoteImages::Blocked => image_host(src).map(|host| ImgSrc::Blocked { host }),
        RemoteImages::Allowed => SafeUrl::parse(src).map(ImgSrc::Remote),
    }
}

fn inline_part(reference: &str, parts: &[ParsedPart], spent: &mut usize) -> Option<Inlined> {
    let wanted = reference
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    if wanted.is_empty() {
        return None;
    }
    let part = parts.iter().find(|part| match &part.inline {
        Inline::Embedded { cid } => cid.trim_start_matches('<').trim_end_matches('>') == wanted,
        Inline::Attached => false,
    })?;
    let mime = embeddable(&part.mime)?;
    let encoded = encoded_len(part.bytes.len());
    if spent.saturating_add(encoded) > INLINE_BUDGET {
        return None;
    }
    *spent += encoded;
    let mut uri = String::with_capacity(encoded + mime.len() + 16);
    uri.push_str("data:");
    uri.push_str(mime);
    uri.push_str(";base64,");
    uri.push_str(&STANDARD.encode(&part.bytes));
    Some(Inlined::new(uri))
}

fn encoded_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

/// A tracking pixel or a layout spacer: a declared side of one pixel or less.
pub(crate) fn is_spacer(width: Option<u32>, height: Option<u32>) -> bool {
    match (width, height) {
        (Some(w), Some(h)) => w <= 1 || h <= 1,
        (Some(w), None) => w <= 1,
        (None, Some(h)) => h <= 1,
        (None, None) => false,
    }
}
