//! Resolving `cid:` references to the message's own inline parts.
//!
//! An HTML mail that embeds its images refers to them as `<img src="cid:logo@example">`. The
//! sanitizer keeps `cid:` deliberately — it names this message's own part and is safe in both
//! image modes — but nothing downstream resolved it, so every inline image rendered broken.
//!
//! `plan.md` asks for a custom protocol handler keyed on `BlobId`. That cannot work here: the
//! reader renders into an iframe with `sandbox=""` and no `allow-same-origin`, which gives the
//! document an opaque origin, and a custom scheme requested from an opaque origin is treated as
//! cross-origin and refused. Adding `allow-same-origin` to make it work would hand every
//! sanitizer bug direct access to the application's DOM, which is the one thing that iframe
//! exists to prevent.
//!
//! So the bytes are inlined as `data:` URIs instead. That satisfies the plan's actual
//! requirement more strongly than the handler would have: the plan forbids a path-shaped handler
//! because it is "a directory-traversal bug driven by untrusted mail", and this resolves nothing
//! at all at request time — there is no handler, no lookup keyed on anything the message says,
//! and no filesystem access. A `cid` from the message is only ever compared against the parts of
//! that same message.

use crate::parse::ParsedPart;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mail_domain::Inline;

/// How many bytes of inline image may be embedded into one message.
///
/// A `data:` URI costs about a third more than the bytes it carries, and the reader holds that
/// URI in the document it draws — in this process, whether as a block or as markup. A message
/// with forty megabytes of inline photographs would stall the renderer, so past this point the
/// remaining references are left as `cid:` — a broken image, which is what they already were.
pub const INLINE_BUDGET: usize = 8 * 1024 * 1024;

/// Media types that may be emitted as a `data:` URI.
///
/// An allowlist, and the security boundary of this module. The media type in the message is
/// attacker-controlled, and `data:text/html` in an `href` is script execution; echoing the
/// declared type would hand that straight through. Only these are emitted, and anything else
/// keeps its `cid:` and renders broken.
///
/// `image/svg+xml` is deliberately absent. An SVG is a document that can carry script, and
/// while `<img src>` does not execute it, nothing here guarantees the reference came from an
/// `<img>` — the sanitizer permits `cid:` wherever a URL is allowed.
const EMBEDDABLE: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Replace `cid:` references in `html` with the bytes of the matching parts.
///
/// Runs on **sanitized** HTML, after `crate::sanitize`. That order is deliberate: the sanitizer
/// decides which URLs survive at all, and this only rewrites ones it already allowed. Running it
/// first would mean handing the sanitizer a `data:` URI to re-judge, and feeding it something it
/// did not see in the original message.
///
/// A reference with no matching part is left alone. Substituting a placeholder would mean the
/// reader inventing content for a message, and leaving it is exactly as broken as it was.
pub fn embed_inline(html: &str, parts: &[ParsedPart], budget: usize) -> String {
    if !html.contains("cid:") {
        return html.to_owned();
    }
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut spent = 0usize;

    while let Some(at) = rest.find("cid:") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "cid:".len()..];
        let end = after
            .find(['"', '\'', '>', ' ', ')', '\t', '\n', '\r'])
            .unwrap_or(after.len());
        let (reference, tail) = after.split_at(end);

        match resolve(reference, parts) {
            // Budget checked against the encoded length, since that is what actually lands in
            // the document.
            Some((mime, bytes)) if spent + encoded_len(bytes.len()) <= budget => {
                spent += encoded_len(bytes.len());
                out.push_str("data:");
                out.push_str(mime);
                out.push_str(";base64,");
                out.push_str(&STANDARD.encode(bytes));
            }
            _ => {
                // Unknown reference, unembeddable type, or out of budget: leave it as it was.
                out.push_str("cid:");
                out.push_str(reference);
            }
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn encoded_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

/// The allowlist's spelling of `declared`, when this module will embed it.
///
/// The message's own spelling is attacker-controlled. Callers emit this string
/// and never the one from the part, so the `data:` writer and the block parser
/// cross the boundary in the same place.
pub fn embeddable(declared: &str) -> Option<&'static str> {
    let primary = declared.split(';').next()?.trim().to_ascii_lowercase();
    EMBEDDABLE.iter().copied().find(|known| *known == primary)
}

/// The part a `cid` names, if it is one this module will embed.
fn resolve<'a>(reference: &str, parts: &'a [ParsedPart]) -> Option<(&'static str, &'a [u8])> {
    // Some senders write `cid:<id>` with the angle brackets a Content-ID header carries.
    let wanted = reference.trim_start_matches('<').trim_end_matches('>');
    if wanted.is_empty() {
        return None;
    }
    let part = parts.iter().find(|part| match &part.inline {
        Inline::Embedded { cid } => cid.trim_start_matches('<').trim_end_matches('>') == wanted,
        Inline::Attached => false,
    })?;
    // The declared type decides only whether we embed, never what we emit: the string written
    // into the document is the matching entry from EMBEDDABLE, not anything from the message.
    let mime = embeddable(&part.mime)?;
    Some((mime, part.bytes.as_slice()))
}
