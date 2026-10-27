//! One message body as the sandboxed frame shows it: sanitized html in a document of the
//! reader's own, or a plain-text message's lines.
//!
//! A pure function of the stored body, the sanitize policy and, for a message OpenPGP or S/MIME
//! opened, the body it opened to. [`super::Frames`] remembers the answers.
//!
//! What a reply quotes of the message it answers is folded behind a small toggle in the
//! document itself ([`super::quote`]), so the reader keeps no state for it.

use crate::error::Logged;
use mail_domain::{Body, Message};
use mail_mime::{SanitizePolicy, escape_html};
use mail_store::SqliteStore;

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
        /// A plain-text message's lines: no ground of its own, so the reader's shows through.
        plain: bool,
        /// The message's own parts the html embeds as `data:` URIs, for [`FrameBody::inline_part`].
        inline: Vec<mail_mime::EmbeddedImage>,
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

    /// The message's own parts this frame embeds as `data:` URIs.
    pub fn embedded(&self) -> &[mail_mime::EmbeddedImage] {
        match self {
            FrameBody::Present { inline, .. } => inline,
            FrameBody::NotFetched => &[],
        }
    }

    /// The `Content-ID` of the message's own part an image in this frame drawn from `src` shows
    /// ([`mail_mime::drawn_from`]). A plain-text message's frame draws no images.
    pub fn inline_part(&self, src: &str) -> Option<String> {
        match self {
            FrameBody::Present { plain: false, .. } => mail_mime::drawn_from(src, self.embedded()),
            _ => None,
        }
    }
}

/// Which of a message's bodies its frame shows when it carries both an HTML and a plain-text one
/// (a `multipart/alternative`). One with only either shows that one, whatever this says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Prefer {
    /// The HTML, as the sender laid it out.
    #[default]
    Html,
    /// The plain text the sender wrote beside it.
    Plain,
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
    prefer: Prefer,
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
            (
                chosen_frame(&parsed, text.as_deref(), policy, prefer),
                source,
            )
        }
    }
}

/// The frame for `parsed`: its HTML, unless `prefer` asks for the plain text and the message has
/// some to show; else its text, or `stored`, the text kept beside the blob. A text part of only
/// whitespace is no text to prefer: the sender put everything in the HTML.
fn chosen_frame(
    parsed: &mail_mime::Parsed,
    stored: Option<&str>,
    policy: SanitizePolicy,
    prefer: Prefer,
) -> FrameBody {
    let text = parsed
        .text
        .as_deref()
        .filter(|text| !text.trim().is_empty());
    match (parsed.html.as_deref(), prefer, text) {
        (Some(_), Prefer::Plain, Some(text)) => plain_frame(text),
        (Some(html), ..) => html_frame(html, parsed, policy),
        (None, ..) => plain_frame(parsed.text.as_deref().or(stored).unwrap_or("")),
    }
}

fn parse_body(store: &SqliteStore, message: &Message) -> Option<mail_mime::Parsed> {
    let raw = message.body.raw()?;
    // A reader, not the writer: a sync's ingest holds the writer for a whole batch.
    let bytes = store
        .blobs()
        .get(raw)
        .or_log("a message's bytes could not be read")?;
    mail_mime::parse(&bytes).ok()
}

fn html_frame(html: &str, parsed: &mail_mime::Parsed, policy: SanitizePolicy) -> FrameBody {
    let safe = mail_mime::sanitize(html, policy);
    let blocked_remote = safe.blocked_remote() > 0;
    let fetches = safe.remote_fetches().to_vec();
    let folded = super::quote::fold_html(safe.as_str());
    let (embedded, inline) =
        mail_mime::embed_inline_listed(&folded, &parsed.attachments, mail_mime::INLINE_BUDGET);
    FrameBody::Present {
        html: frame_document(&html_sheet(), safe.body_style(), &embedded),
        blocked_remote,
        fetches,
        plain: false,
        inline,
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
/// The dark scheme's `--ink`, for a plain-text message's lines.
const FRAME_INK_DARK: &str = "#ececec";
const FRAME_PAPER: &str = "#ffffff";
const FRAME_CODE_GROUND: &str = "#f5f5f5";
const FRAME_LINK: &str = "#0066cc";

/// What an HTML message's document starts from before the sender's own sheets: a browser's
/// defaults, as a mail client's are, with quotes, code and headings drawn as the composer draws
/// them, so a message written here reads as it was written, with nothing wider than the frame where Blitz can say so
/// (a table's own percentage `max-width` does not hold a fixed-width table nested in an
/// auto-width one: what such a newsletter lays out wider than the pane the reader then draws
/// smaller, `mail-app`'s `reading/fit.rs`). Neither rule
/// that does so makes a table cell narrower than a browser would: a long word wraps only where a
/// line overflows (`break-word`, not `anywhere`, which lets a cell squeeze "iOS" to a letter a
/// line), and an image shrinks to the frame but no icon to nothing (`max(100%, 64px)`: a bare
/// percentage counts as nothing when a table sizes its columns). An image keeps the size its
/// attributes give it, so a blocked one still holds its place. The sender's
/// `<style>` comes later in the document and its `<body>` colours and style sit on the body
/// itself, so either wins, and a sender's ground still fills the frame to its edges. The gutter
/// the words are read in is not the body's: it is the inset's ([`INSET_STYLE`]), which no
/// `padding: 0` of the sender's reaches.
fn html_sheet() -> String {
    format!(
        ":root {{ color-scheme: light; }} html, body {{ margin: 0; }} \
         body {{ padding: 0; font-family: {FRAME_FONT}; font-size: {FRAME_TEXT_SIZE}; \
         line-height: {FRAME_TEXT_LINE}; color: {FRAME_INK}; background: {FRAME_PAPER}; \
         overflow-wrap: break-word; }} \
         img {{ max-width: max(100%, 64px); }} table {{ max-width: 100%; }} \
         blockquote {{ margin: 0 0 1em; padding-left: 12px; \
         border-left: {FRAME_QUOTE_BAR} solid {FRAME_INK_SOFT}; color: {FRAME_INK_SOFT}; }} \
         pre, code {{ font-family: {FRAME_CODE_FONT}; font-size: {FRAME_CODE_SIZE}; }} \
         pre {{ background: {FRAME_CODE_GROUND}; border-radius: 6px; padding: 10px 12px; white-space: pre-wrap; }} \
         h2 {{ font-size: 1.3em; line-height: 1.25; margin: .9em 0 .35em; }} \
         h3 {{ font-size: 1.15em; margin: .8em 0 .3em; }} h4 {{ font-size: 1em; margin: .7em 0 .3em; }} \
         a {{ color: {FRAME_LINK}; }} {}",
        super::quote::sheet(FRAME_INK_SOFT),
    )
}

/// The class of the element a frame's words are read in, inside its `<body>` ([`frame_document`]).
/// The sanitizer keeps a sender's markup from carrying it and a sender's sheet from naming it
/// (`mail_mime::sanitize`: no class, id or selector of the sender's starts `mailo-`).
const INSET_CLASS: &str = "mailo-inset";

/// The inset's style (padding 18px, the reader's `--s-18`, the gutter its header and every other
/// part of a message are drawn at, so a message's first line starts under the sender's name),
/// the same for an HTML message and a plain-text one, so every message's
/// words line up. On an element of the frame's own, never the sender's `<body>`: Gmail and Mail
/// keep a gutter whatever a sender's body says, and a sender's `margin: 0; padding: 0` on the
/// body is for a browser window, where the window's edge is the page's. It is written inline on
/// the element with `!important` on every property that shapes the gutter, so no rule of the
/// sender's, however it is selected (`div:not(#z)`, `body > *`), outweighs it, and
/// `border-box` with `width: auto` keeps the padding inside the frame. The body around it keeps
/// the sender's ground, which still fills the frame.
const INSET_STYLE: &str = "display:block!important;box-sizing:border-box!important;\
    width:auto!important;max-width:none!important;min-width:0!important;float:none!important;\
    position:static!important;margin:0!important;border:0!important;padding:18px!important";

/// A plain-text message's sheet: its lines as written, on no ground of their own, so the
/// reader's shows through, in the reader's ink for the scheme the window is in (a frame's
/// document takes the window's scheme). The inset is on the element ([`INSET_STYLE`]).
fn plain_sheet() -> String {
    format!(
        ":root {{ color-scheme: light dark; }} html, body {{ background: transparent; }} \
         body {{ margin: 0; font-family: {FRAME_FONT}; font-size: {FRAME_TEXT_SIZE}; \
         line-height: {FRAME_TEXT_LINE}; color: {FRAME_INK}; white-space: pre-wrap; \
         overflow-wrap: anywhere; }} {} \
         @media (prefers-color-scheme: dark) {{ body {{ color: {FRAME_INK_DARK}; }} \
         details.mailo-quote > summary {{ color: {FRAME_INK_DARK}; }} }}",
        super::quote::sheet(FRAME_INK_SOFT),
    )
}

/// A frame's whole document: `sheet`, then `body` (markup already safe to show) in the inset
/// ([`INSET_CLASS`]), in a `<body>` carrying `body_style`, the sender's own body colours and
/// style, when there are any.
fn frame_document(sheet: &str, body_style: Option<&str>, body: &str) -> String {
    let styled = body_style
        .map(|style| format!(" style=\"{}\"", escape_html(style)))
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>{sheet}</style></head>\
         <body{styled}><div class=\"{INSET_CLASS}\" style=\"{INSET_STYLE}\">{body}</div></body></html>"
    )
}

fn plain_frame(text: &str) -> FrameBody {
    FrameBody::Present {
        html: frame_document(
            &plain_sheet(),
            None,
            &super::quote::fold_plain(text, escape_html),
        ),
        blocked_remote: false,
        fetches: Vec::new(),
        plain: true,
        inline: Vec::new(),
    }
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
            page.contains(&format!(
                "<body style=\"background-color:#f3f1ec;padding:0\">\
                 <div class=\"mailo-inset\" style=\"{INSET_STYLE}\"><p>Hi</p></div></body>"
            )),
            "{page}"
        );
        assert!(!html_sheet().contains("height: auto"), "{}", html_sheet());
    }

    /// Every message's words sit in the frame's own inset, 18 px in in an HTML frame and a
    /// plain one alike, and a sender can neither carry the inset's class nor name it in a sheet:
    /// a notice's `<body style="margin:0;padding:0">` (the owner's sample's) keeps its ground on
    /// the body and loses nothing of the gutter.
    #[test]
    fn the_words_sit_in_an_inset_the_sender_cannot_reach() {
        assert!(INSET_STYLE.contains("padding:18px!important"));
        for sheet in [html_sheet(), plain_sheet()] {
            assert!(!sheet.contains("padding: 18px"), "{sheet}");
            assert!(!sheet.contains("margin: 18px"), "{sheet}");
        }
        let raw = include_str!("../../tests/fixtures/flush-notice.eml");
        let parsed = mail_mime::parse(raw.as_bytes()).expect("the notice parses");
        let html = parsed.html.as_deref().expect("its HTML part");
        let body = html_frame(html, &parsed, SanitizePolicy::FRAME);
        let page = body.frame_html().expect("present");
        assert!(
            page.contains("<body style=\"margin:0;padding:0;background-color:#ffffff;")
                && page.contains(&format!(
                    "><div class=\"mailo-inset\" style=\"{INSET_STYLE}\">"
                )),
            "{page}"
        );
        assert!(page.ends_with("</div></body></html>"), "{page}");

        let posing = "<style>.mailo-inset{padding:0!important} #mailo-inset, \
                      [class*=inset-mailo]{margin:0} div:not(#z){padding:0!important;width:100%!important}</style>\
                      <div class=\"mailo-inset lead\" id=\"mailo-inset\">Hi</div>";
        let parsed =
            mail_mime::parse(format!("Content-Type: text/html\r\n\r\n{posing}").as_bytes())
                .expect("parses");
        let body = html_frame(
            parsed.html.as_deref().expect("html"),
            &parsed,
            SanitizePolicy::FRAME,
        );
        let page = body.frame_html().expect("present");
        assert_eq!(
            page.matches("class=\"mailo-inset").count(),
            1,
            "only the frame's own inset carries the class: {page}"
        );
        assert!(!page.contains("id=\"mailo-inset"), "{page}");
        assert!(page.contains("<div class=\"lead\">Hi</div>"), "{page}");
        assert!(
            page.contains(
                ".x-mailo-inset{padding:0!important} #x-mailo-inset, [x-mailo]{margin:0}"
            ),
            "{page}"
        );
    }

    /// Nothing a sender sizes small in a table is squeezed past what a browser draws: a word
    /// is broken only where its line overflows, and an icon in a cell keeps its size while a
    /// large image still shrinks to the frame (a Teams mail's "iOS" button drew its label a letter
    /// a line, with no icon).
    #[test]
    fn the_sheet_keeps_a_table_cell_as_wide_as_a_browser_would() {
        let sheet = html_sheet();
        assert!(sheet.contains("overflow-wrap: break-word;"), "{sheet}");
        assert!(!sheet.contains("anywhere"), "{sheet}");
        assert!(
            sheet.contains("img { max-width: max(100%, 64px); }"),
            "{sheet}"
        );
    }

    /// A reply's quoted tail reaches both frames folded, behind the toggle.
    #[test]
    fn a_quoted_tail_is_folded_in_either_frame() {
        let FrameBody::Present { html, .. } = plain_frame("Yes.\n\nOn Mon, Ada wrote:\n> Ten?\n")
        else {
            panic!("a plain frame is present");
        };
        assert!(
            html.contains("Yes.\n\n<details class=\"mailo-quote\">")
                && html.contains("On Mon, Ada wrote:\n&gt; Ten?\n</div></details>"),
            "{html}"
        );
        let parsed = mail_mime::parse(
            b"Content-Type: text/html\r\n\r\n<p>Yes.</p><blockquote>Ten?</blockquote>",
        )
        .unwrap();
        let FrameBody::Present { html, .. } = html_frame(
            parsed.html.as_deref().unwrap(),
            &parsed,
            SanitizePolicy::FRAME,
        ) else {
            panic!("an html frame is present");
        };
        assert!(
            html.contains("<details class=\"mailo-quote\">")
                && html.contains("<blockquote>Ten?</blockquote></div></details>"),
            "{html}"
        );
    }

    /// Which body a frame shows, by what the message carries and what the reader prefers: the
    /// plain text only where it is asked for and there is some.
    #[test]
    fn plain_text_is_shown_only_where_it_is_preferred_and_there_is_some() {
        let alternative = "MIME-Version: 1.0\r\n\
             Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nPlain words\r\n\
             --b\r\nContent-Type: text/html\r\n\r\n<p>Rich words</p>\r\n--b--\r\n";
        let blank_text = "MIME-Version: 1.0\r\n\
             Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\n \r\n\
             --b\r\nContent-Type: text/html\r\n\r\n<p>Rich words</p>\r\n--b--\r\n";
        let html_only = "MIME-Version: 1.0\r\nContent-Type: text/html\r\n\r\n<p>Rich words</p>\r\n";
        let text_only = "Content-Type: text/plain\r\n\r\nPlain words\r\n";
        // (case, message, preferred, shown as plain text, the words shown)
        let cases = [
            ("both, html", alternative, Prefer::Html, false, "Rich words"),
            (
                "both, plain",
                alternative,
                Prefer::Plain,
                true,
                "Plain words",
            ),
            ("blank text", blank_text, Prefer::Plain, false, "Rich words"),
            ("html only", html_only, Prefer::Plain, false, "Rich words"),
            (
                "text only, html",
                text_only,
                Prefer::Html,
                true,
                "Plain words",
            ),
            (
                "text only, plain",
                text_only,
                Prefer::Plain,
                true,
                "Plain words",
            ),
        ];
        for (case, raw, prefer, plain_wanted, words) in cases {
            let parsed = mail_mime::parse(raw.as_bytes()).expect("a message");
            let FrameBody::Present { html, plain, .. } =
                chosen_frame(&parsed, None, SanitizePolicy::FRAME, prefer)
            else {
                panic!("{case}: a frame is present");
            };
            assert_eq!(plain, plain_wanted, "{case}");
            assert!(html.contains(words), "{case}: {html}");
        }
    }

    #[test]
    fn text_is_escaped_into_the_plain_frame() {
        let FrameBody::Present { html, .. } = plain_frame("a <b> & \"c\"") else {
            panic!("a plain frame is present");
        };
        assert!(html.contains("a &lt;b&gt; &amp; &quot;c&quot;"), "{html}");
    }

    /// An image the frame embedded reads back as its part, by the URI the frame reports it with
    /// (parsed as a URL, as the renderer resolves it); a reference left as `cid:` reads as the
    /// part it names; a remote image, or a `data:` URI the sender wrote, reads as nothing.
    #[test]
    fn an_embedded_image_reads_back_as_its_part() {
        let raw = "From: a@example.test\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/related; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/html\r\n\r\n\
             <img src=\"cid:logo@x\"><img src=\"cid:gone@x\">\
             <img src=\"https://cdn.example.test/a.png\">\r\n\
             --b\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\n\
             Content-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b--\r\n";
        let parsed = mail_mime::parse(raw.as_bytes()).expect("a message");
        let body = html_frame(
            parsed.html.as_deref().expect("html"),
            &parsed,
            SanitizePolicy::FRAME,
        );
        let html = body.frame_html().expect("present");
        let at = html.find("data:image/png;base64,").expect("embedded");
        let uri = &html[at..at + html[at..].find('"').expect("closed")];
        assert_eq!(body.inline_part(uri).as_deref(), Some("logo@x"));
        let resolved = url::Url::parse(uri).expect("a URL").to_string();
        assert_eq!(body.inline_part(&resolved).as_deref(), Some("logo@x"));
        assert_eq!(body.inline_part("cid:<gone@x>").as_deref(), Some("gone@x"));
        assert_eq!(body.inline_part("https://cdn.example.test/a.png"), None);
        assert_eq!(body.inline_part("data:image/png;base64,R0lGODlh"), None);
        assert_eq!(plain_frame("cid:logo@x").inline_part("cid:logo@x"), None);
    }
}
