//! One message body as the sandboxed frame shows it: sanitized html in a document of the
//! reader's own, or a plain-text message's lines.
//!
//! A pure function of the stored body, the sanitize policy and, for a message OpenPGP or S/MIME
//! opened, the body it opened to. [`super::Frames`] remembers the answers.

use mail_domain::{Body, Message};
use mail_mime::SanitizePolicy;
use mail_store::{SqliteStore, Store};

/// What the reader displays for one message body in its sandboxed frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameBody {
    /// Headers only so far. Normal mid-sync, and not an empty message.
    NotFetched,
    /// The body rendered as sanitized HTML.
    Present {
        html: String,
        blocked_remote: bool,
        fetches: Vec<String>,
    },
}

impl FrameBody {
    pub fn frame_html(&self) -> Option<&str> {
        match self {
            FrameBody::Present { html, .. } => Some(html),
            FrameBody::NotFetched => None,
        }
    }

    pub fn blocked_remote(&self) -> bool {
        match self {
            FrameBody::Present { blocked_remote, .. } => *blocked_remote,
            FrameBody::NotFetched => false,
        }
    }

    pub fn frame_fetches(&self) -> &[String] {
        match self {
            FrameBody::Present { fetches, .. } => fetches,
            FrameBody::NotFetched => &[],
        }
    }
}

/// What a rendering was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The stored body: the blob, or the text beside it.
    Stored,
    /// A body OpenPGP or S/MIME opened. A function of more than the blob, and of a key that may
    /// be locked again, so nothing keyed by the blob may keep it.
    Opened,
}

/// Render a message body into its sandboxed frame, every time, saying what it rendered from —
/// asked once, here, so a seal opened while it renders cannot leave a cache believing the
/// plaintext is the blob's.
///
/// `unsealed` is the body OpenPGP or S/MIME opened `message` to, when it has been opened. The
/// reader goes through [`super::Frames::rendered`], which remembers the answer; this is the work
/// itself, for the cache to call and for a measurement that must not hit it.
pub fn render(
    store: &SqliteStore,
    message: &Message,
    policy: SanitizePolicy,
    unsealed: &dyn Fn(&Message) -> Option<mail_mime::Parsed>,
) -> (FrameBody, Source) {
    match &message.body {
        Body::Absent => (FrameBody::NotFetched, Source::Stored),
        Body::Present { text, .. } => {
            let (parsed, source) = match unsealed(message) {
                Some(opened) => (Some(opened), Source::Opened),
                None => (parse_body(store, message), Source::Stored),
            };
            let Some(parsed) = parsed else {
                return (plain_frame(text.as_deref().unwrap_or("")), source);
            };
            let body = if let Some(html) = parsed.html.as_deref() {
                html_frame(html, &parsed, policy)
            } else {
                let shown = parsed.text.as_deref().or(text.as_deref()).unwrap_or("");
                plain_frame(shown)
            };
            (body, source)
        }
    }
}

fn parse_body(store: &SqliteStore, message: &Message) -> Option<mail_mime::Parsed> {
    let raw = message.body.raw()?;
    // A reader, not the writer: a sync's ingest holds the writer for a whole batch.
    let bytes = store.blobs().get(raw).ok()?;
    mail_mime::parse(&bytes).ok()
}

fn html_frame(html: &str, parsed: &mail_mime::Parsed, policy: SanitizePolicy) -> FrameBody {
    let safe = mail_mime::sanitize(html, policy);
    let blocked_remote = safe.blocked_remote() > 0;
    let fetches = safe.remote_fetches().to_vec();
    let embedded =
        mail_mime::embed_inline(safe.as_str(), &parsed.attachments, mail_mime::INLINE_BUDGET);
    FrameBody::Present {
        html: frame_document(&html_sheet(), safe.body_style(), &embedded),
        blocked_remote,
        fetches,
    }
}

/// The type a frame's document is set in, a mail client's own: the system's sans rather than
/// Blitz's serif.
const FRAME_FONT: &str = "\"Inter\", system-ui, sans-serif";

/// The document inside a frame cannot read the window's tokens, so the values the window's rules
/// name are written here once, as named constants, and the sheets below are built from them: the
/// message text's size and line height (`--fs-reading`, 1.65), the code face and size
/// (`--font-code`, `--fs-meta`) and the quote bar (2 px, `--ink-soft`). The light colours are the
/// light scheme's `--ink`, `--ink-soft` and `--surface`.
const FRAME_TEXT_SIZE: &str = "14px";
const FRAME_TEXT_LINE: &str = "1.65";
const FRAME_CODE_FONT: &str = "\"Space Mono\", \"Inter\", ui-monospace, monospace";
const FRAME_CODE_SIZE: &str = "12.5px";
const FRAME_QUOTE_BAR: &str = "2px";
const FRAME_INK: &str = "#202020";
const FRAME_INK_SOFT: &str = "#5c5c5c";
const FRAME_PAPER: &str = "#ffffff";
const FRAME_CODE_GROUND: &str = "#f5f5f5";
const FRAME_LINK: &str = "#0066cc";

/// What an HTML message's document starts from before the sender's own sheets: a browser's
/// defaults, as a mail client's are, with quotes, code and headings drawn as the composer draws
/// them, so a message written here reads as it was written, with nothing wider than the frame where Blitz can say so
/// (a table's own percentage `max-width` does not hold a fixed-width table nested in an
/// auto-width one: a pane under 600 px still cuts a 600 px newsletter's right edge). An image
/// keeps the size its attributes give it, so a blocked one still holds its place. The sender's
/// `<style>` comes later in the document and its `<body>` colours and style sit on the body
/// itself, so either wins, a `padding: 0` included, and a full-bleed design stays one.
fn html_sheet() -> String {
    format!(
        ":root {{ color-scheme: light; }} html, body {{ margin: 0; }} \
         body {{ padding: 16px; font-family: {FRAME_FONT}; font-size: {FRAME_TEXT_SIZE}; \
         line-height: {FRAME_TEXT_LINE}; color: {FRAME_INK}; background: {FRAME_PAPER}; \
         overflow-wrap: anywhere; }} \
         img {{ max-width: 100%; }} table {{ max-width: 100%; }} \
         blockquote {{ margin: 0 0 1em; padding-left: 12px; \
         border-left: {FRAME_QUOTE_BAR} solid {FRAME_INK_SOFT}; color: {FRAME_INK_SOFT}; }} \
         pre, code {{ font-family: {FRAME_CODE_FONT}; font-size: {FRAME_CODE_SIZE}; }} \
         pre {{ background: {FRAME_CODE_GROUND}; border-radius: 6px; padding: 10px 12px; white-space: pre-wrap; }} \
         h2 {{ font-size: 1.3em; line-height: 1.25; margin: .9em 0 .35em; }} \
         h3 {{ font-size: 1.15em; margin: .8em 0 .3em; }} h4 {{ font-size: 1em; margin: .7em 0 .3em; }} \
         a {{ color: {FRAME_LINK}; }}"
    )
}

/// A plain-text message's sheet: its lines as written, in the scheme the desktop is in.
fn plain_sheet() -> String {
    format!(
        ":root {{ color-scheme: light dark; }} \
         body {{ margin: 16px; font-family: {FRAME_FONT}; font-size: {FRAME_TEXT_SIZE}; \
         line-height: {FRAME_TEXT_LINE}; white-space: pre-wrap; overflow-wrap: anywhere; }}"
    )
}

/// A frame's whole document: `sheet`, then `body` (markup already safe to show) in a `<body>`
/// carrying `body_style`, the sender's own body colours and style, when there are any.
fn frame_document(sheet: &str, body_style: Option<&str>, body: &str) -> String {
    let styled = body_style
        .map(|style| format!(" style=\"{}\"", escape_html(style)))
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>{sheet}</style></head>\
         <body{styled}>{body}</body></html>"
    )
}

fn plain_frame(text: &str) -> FrameBody {
    FrameBody::Present {
        html: frame_document(&plain_sheet(), None, &escape_html(text)),
        blocked_remote: false,
        fetches: Vec::new(),
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' | '\t' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sender's `<body>` colours reach the frame's own body, after the base sheet, so they
    /// win over its white; and the base sheet leaves an image's size to its attributes.
    #[test]
    fn the_sender_s_body_style_is_the_frame_s_body() {
        let safe = mail_mime::sanitize(
            "<body bgcolor=\"#f3f1ec\" style=\"padding:0\"><p>Hi</p></body>",
            SanitizePolicy::FRAME,
        );
        let page = frame_document(&html_sheet(), safe.body_style(), safe.as_str());
        assert!(
            page.contains("<body style=\"background-color:#f3f1ec;padding:0\"><p>Hi</p>"),
            "{page}"
        );
        assert!(!html_sheet().contains("height: auto"), "{}", html_sheet());
    }

    #[test]
    fn text_is_escaped_into_the_plain_frame() {
        let FrameBody::Present { html, .. } = plain_frame("a <b> & \"c\"") else {
            panic!("a plain frame is present");
        };
        assert!(html.contains("a &lt;b&gt; &amp; &quot;c&quot;"), "{html}");
    }
}
