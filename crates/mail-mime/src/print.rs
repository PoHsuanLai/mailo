//! A message, or a whole thread, as one printable HTML document.
//!
//! The body takes the reader's path: an HTML part is sanitized with remote images blocked and
//! lowered to blocks, and plain text is lowered to the same blocks. The document is then
//! written from those blocks by this module and nothing else. No byte of the message's own
//! markup reaches the output — only what a [`Block`] can say, every piece of text escaped here —
//! so a script, an event handler, a `<meta refresh>`, a form or a stylesheet has no way through
//! even if the sanitizer missed it.
//!
//! Self-contained on purpose: printing happens from a file on disk or a webview with the
//! network nowhere in the picture, and a printout that fetched something would tell the sender
//! it had been printed. The only URLs that load anything are `data:` URIs: those this crate
//! built from the message's own parts, through [`crate::embeddable`]'s allowlist, and, for a
//! message whose remote images the reader consented to ([`Remote::Allowed`]), those the caller
//! fetched already and hands in, held to the same allowlist. Nothing here fetches.
//!
//! The document also says, in markers a paginating renderer reads, what the page structure is
//! and what each message is written in, so no one has to edit the markup afterwards:
//! - `data-break-before="page"` on a message that starts a page ([`Pages::PerMessage`]);
//! - `data-break-inside="avoid"` on a message's headers, a quoted block, a table, an image and
//!   the list of attachments: what reads badly cut in two by a page's end;
//! - `data-script` on a message whose CJK script its own headers or text say ([`Script`]), so
//!   a renderer can choose the regional face for it.
//!
//! The same attributes mean nothing to a browser; the stylesheet's `break-*` rules say the same
//! things to one that reads CSS fragmentation.
//!
//! Pure: the time zone and "now" are arguments, and nothing here reads the clock or the disk.

use crate::block::{
    Block, Dir, Document, ImgSrc, Reached, Span, from_html_describing, from_text_keeping_lines,
};
use crate::parse::Parsed;
use crate::sanitize::{RemoteImages, SanitizePolicy, sanitize};
use crate::script::{Script, script_of};
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{Address, Body, Message};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::fmt::Write as _;

/// One message to print, and what its stored bytes said.
///
/// `parsed` is `None` when the body has not been fetched, when its bytes are gone, or when
/// they do not parse — the same three cases the reader falls back to the stored text for.
#[derive(Debug, Clone, Copy)]
pub struct Sheet<'a> {
    pub message: &'a Message,
    pub parsed: Option<&'a Parsed>,
    /// The remote images this message's printout may draw.
    pub remote: Remote<'a>,
}

/// The remote images a message's printout may draw. A remote image is a read receipt: only the
/// reader's consent to this message's images lets one in, and then only as bytes the caller
/// already fetched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Remote<'a> {
    /// None: each is named where it stood, by its description and its host. Always, without
    /// the reader's consent.
    #[default]
    Blocked,
    /// The reader's consent stands for this message. Each image in the map (its URL, as
    /// [`remote_images`] listed it, to a `data:` URI of an allowlisted raster type) is drawn;
    /// one missing from it, or whose URI is not such a `data:` URI, is named as a blocked one is.
    Allowed(&'a BTreeMap<String, String>),
}

/// How the document is made beyond what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options<'a> {
    pub pages: Pages,
    /// Rules written after the document's own stylesheet: a renderer's named faces, say. The
    /// caller's, written as given, and never anything a message said.
    pub style: &'a str,
    /// A line under the "Printed" line when the printout names an image rather than drawing
    /// it. Escaped like every other text.
    pub missing_note: Option<&'a str>,
}

impl Options<'_> {
    /// `pages`, and nothing more: the document's own stylesheet and no note.
    pub fn new(pages: Pages) -> Options<'static> {
        Options {
            pages,
            style: "",
            missing_note: None,
        }
    }
}

/// Whether each message of a thread starts on a new page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pages {
    /// One after another, a rule between them. Saves paper on a thread of short replies.
    Flow,
    /// Every message after the first starts a new page.
    PerMessage,
}

/// The printable document for `sheets`, in the order given.
///
/// The caller orders a thread (oldest first reads as a conversation). Dates are shown in
/// `zone`; `now` is the "printed" line at the top. The title is the first message's subject.
pub fn print<Tz>(sheets: &[Sheet<'_>], zone: &Tz, now: DateTime<Utc>, pages: Pages) -> String
where
    Tz: TimeZone,
    Tz::Offset: Display,
{
    print_with(sheets, zone, now, &Options::new(pages))
}

/// [`print`], made as `options` say: the pages, rules after the stylesheet, and a note when an
/// image is named rather than drawn.
pub fn print_with<Tz>(
    sheets: &[Sheet<'_>],
    zone: &Tz,
    now: DateTime<Utc>,
    options: &Options<'_>,
) -> String
where
    Tz: TimeZone,
    Tz::Offset: Display,
{
    // The messages first, so the top can say whether an image in them was left out.
    let mut messages = String::new();
    for (index, sheet) in sheets.iter().enumerate() {
        let (class, starts_page) = match (index, options.pages) {
            (0, _) | (_, Pages::Flow) => ("message", ""),
            (_, Pages::PerMessage) => ("message new-page", " data-break-before=\"page\""),
        };
        let script = script(sheet)
            .map(|script| format!(" data-script=\"{}\"", script.tag()))
            .unwrap_or_default();
        let _ = writeln!(messages, "<article class=\"{class}\"{starts_page}{script}>");
        header(&mut messages, sheet.message, zone);
        body(&mut messages, sheet);
        attachments(&mut messages, sheet.message);
        messages.push_str("</article>\n");
    }
    // Every text is escaped, so this element can only be one `missing` wrote.
    let left_out = messages.contains(MISSING);
    let title = sheets
        .first()
        .map(|sheet| sheet.message.subject.trim())
        .filter(|subject| !subject.is_empty())
        .unwrap_or("(no subject)");
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    // A second line of defence behind "we emit nothing that loads": should a future edit let
    // something through, the document itself still refuses to fetch or run it.
    out.push_str(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         img-src data:; style-src 'unsafe-inline'\">\n",
    );
    let _ = writeln!(out, "<title>{}</title>", escape(title));
    out.push_str("<style>\n");
    out.push_str(CSS);
    if !options.style.is_empty() {
        out.push_str(options.style);
        if !options.style.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("</style>\n</head>\n<body>\n");
    let _ = writeln!(
        out,
        "<p class=\"printed\">Printed {}</p>",
        escape(&when(now, zone))
    );
    if left_out && let Some(note) = options.missing_note {
        let _ = writeln!(out, "<p class=\"paper-note\">{}</p>", escape(note));
    }
    if sheets.len() > 1 {
        let _ = writeln!(
            out,
            "<h1 class=\"thread\">{} <span class=\"count\">({} messages)</span></h1>",
            escape(title),
            sheets.len()
        );
    }
    out.push_str(&messages);
    out.push_str("</body>\n</html>\n");
    out
}

/// The CJK script `sheet` is written in, as far as the message says: its own headers when its
/// bytes are here, then its subject and its text.
fn script(sheet: &Sheet<'_>) -> Option<Script> {
    let subject = sheet.message.subject.as_str();
    match sheet.parsed {
        Some(parsed) => parsed.script(subject),
        None => {
            let stored = match &sheet.message.body {
                Body::Present { text, .. } => text.as_deref().unwrap_or(""),
                _ => "",
            };
            script_of(None, None, &format!("{subject}\n{stored}"))
        }
    }
}

/// The remote images [`print`] would draw for `sheets` if it had them: the `http` and `https`
/// URLs of every image in the messages whose images are [`Remote::Allowed`], in order, each
/// once. A spacer (a side of one pixel or less) is left out: it is drawn as nothing. A message
/// without consent contributes nothing, so a caller that fetches exactly this list never asks
/// for an image nobody allowed.
pub fn remote_images(sheets: &[Sheet<'_>]) -> Vec<String> {
    let mut urls = Vec::new();
    for sheet in sheets {
        if sheet.remote == Remote::Blocked {
            continue;
        }
        if let Some(document) = document(sheet) {
            collect_remote(&document.blocks, &mut urls);
        }
    }
    urls
}

fn collect_remote(blocks: &[Block], urls: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Image {
                src: ImgSrc::Remote(url),
                width,
                height,
                ..
            } if !is_spacer(*width, *height) => {
                let url = url.as_str();
                let web = url.starts_with("https://") || url.starts_with("http://");
                if web && !urls.iter().any(|seen| seen == url) {
                    urls.push(url.to_owned());
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    collect_remote(item, urls);
                }
            }
            Block::Quote { blocks, .. } | Block::Signature(blocks) => collect_remote(blocks, urls),
            _ => {}
        }
    }
}

/// A `data:` URI this document may draw: base64 of one of [`crate::embeddable`]'s raster types,
/// and nothing that could end the attribute or say anything else.
fn drawable(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("data:") else {
        return false;
    };
    let Some((mime, data)) = rest.split_once(";base64,") else {
        return false;
    };
    crate::embeddable(mime) == Some(mime)
        && !data.is_empty()
        && data
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// How [`missing`] opens its line.
const MISSING: &str = "<p class=\"missing\">";

/// A4 and Letter alike: the margins fit both, and nothing is sized to either sheet.
const CSS: &str = "\
@page { margin: 18mm 16mm; }
html { color: #000; background: #fff; }
body { font: 11pt/1.45 Georgia, 'Times New Roman', serif; margin: 0 auto; max-width: 48em; }
.printed { font: 8pt sans-serif; color: #555; text-align: right; margin: 0 0 1em; }
h1.thread { font: bold 15pt sans-serif; margin: 0 0 1em; }
h1.thread .count { font-weight: normal; color: #555; font-size: 10pt; }
.message + .message { border-top: 1px solid #999; margin-top: 1.5em; padding-top: 1em; }
.message.new-page { break-before: page; page-break-before: always; border-top: none; }
.headers { break-inside: avoid; page-break-inside: avoid; break-after: avoid; \
page-break-after: avoid; font: 9.5pt/1.4 sans-serif; border-bottom: 1px solid #ccc; \
padding-bottom: .6em; margin-bottom: 1em; }
.headers h2 { font: bold 13pt/1.3 sans-serif; margin: 0 0 .4em; }
.headers table { border-collapse: collapse; }
.headers th { text-align: left; vertical-align: top; color: #555; font-weight: normal; \
padding: 0 .8em 0 0; white-space: nowrap; }
.headers td { padding: 0; overflow-wrap: anywhere; }
.body { overflow-wrap: anywhere; }
.body p { margin: 0 0 .8em; }
.body pre { white-space: pre-wrap; font: 9.5pt/1.35 monospace; overflow-wrap: anywhere; }
.body code { font-family: monospace; }
.body blockquote { margin: 0 0 .8em .2em; padding-left: .9em; border-left: 3px solid #999; \
color: #333; }
.body .attribution { color: #555; margin-bottom: .3em; }
.body img { max-width: 100%; height: auto; }
.body table { border-collapse: collapse; margin: 0 0 .8em; }
.body td, .body th { border: 1px solid #bbb; padding: .2em .5em; vertical-align: top; }
.body dl { display: grid; grid-template-columns: max-content auto; gap: .2em 1em; }
.body dd { margin: 0; }
.body .signature { color: #555; font-size: 9.5pt; margin-top: 1em; }
.body .missing { color: #555; font-style: italic; }
.body .url { color: #555; font-size: 9pt; overflow-wrap: anywhere; }
a { color: inherit; }
.note { font: italic 10pt sans-serif; color: #555; }
.attachments { break-inside: avoid; page-break-inside: avoid; font: 9.5pt sans-serif; \
border-top: 1px solid #ccc; margin-top: 1em; padding-top: .5em; }
.attachments ul { margin: .3em 0 0; padding-left: 1.2em; }
.attachments .size { color: #555; }
";

fn header<Tz>(out: &mut String, message: &Message, zone: &Tz)
where
    Tz: TimeZone,
    Tz::Offset: Display,
{
    let _ = writeln!(out, "<header class=\"headers\"{KEEP}>");
    let subject = message.subject.trim();
    let subject = if subject.is_empty() {
        "(no subject)"
    } else {
        subject
    };
    let _ = writeln!(out, "<h2>{}</h2>", escape(subject));
    out.push_str("<table>\n");
    row(out, "From", &addresses(std::slice::from_ref(&message.from)));
    if !message.to.is_empty() {
        row(out, "To", &addresses(&message.to));
    }
    if !message.cc.is_empty() {
        row(out, "Cc", &addresses(&message.cc));
    }
    row(out, "Date", &when(message.date, zone));
    row(out, "Subject", subject);
    out.push_str("</table>\n</header>\n");
}

fn row(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(
        out,
        "<tr><th>{}</th><td>{}</td></tr>",
        escape(label),
        escape(value)
    );
}

/// `Ada Lovelace <ada@example.org>`, or the bare address when there is no name.
fn addresses(list: &[Address]) -> String {
    list.iter()
        .map(|address| match address.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => format!("{name} <{}>", address.email),
            _ => address.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A date as a header shows it, in the reader's zone: `Thu, 24 Sep 2026 14:05 +08:00`.
fn when<Tz>(instant: DateTime<Utc>, zone: &Tz) -> String
where
    Tz: TimeZone,
    Tz::Offset: Display,
{
    instant
        .with_timezone(zone)
        .format("%a, %-d %b %Y %H:%M %:z")
        .to_string()
}

/// The reader's choice of body, as blocks: see `mail-app`'s `view::reading`, which this
/// follows case for case. The one difference is the image a message refers to and does not
/// hold, which the reader drops and a printout names, and a fixed line of plain text, which
/// the reader rewraps and a printout ends where the sender did.
fn document(sheet: &Sheet<'_>) -> Option<Document> {
    let Body::Present { text, .. } = &sheet.message.body else {
        return None;
    };
    let stored = text.as_deref().unwrap_or("");
    let Some(parsed) = sheet.parsed else {
        return Some(from_text_keeping_lines(stored, crate::block::Flowed::Fixed));
    };
    Some(match parsed.html.as_deref() {
        Some(html) => {
            // Sanitized with remote URLs kept, then lowered with them blocked: the reader's
            // two steps. The block parser keeps only the host of a blocked image, so the
            // printout can say whose image is missing without holding an address to fetch.
            // With the reader's consent the URLs stay, to be matched to what was fetched.
            let policy = SanitizePolicy {
                remote_images: RemoteImages::Allowed,
                ..SanitizePolicy::CURRENT
            };
            let safe = sanitize(html, policy);
            let lowered = match sheet.remote {
                Remote::Blocked => RemoteImages::Blocked,
                Remote::Allowed(_) => RemoteImages::Allowed,
            };
            from_html_describing(&safe, &parsed.attachments, lowered)
        }
        None => from_text_keeping_lines(parsed.text.as_deref().unwrap_or(stored), parsed.flowed),
    })
}

fn body(out: &mut String, sheet: &Sheet<'_>) {
    let Some(document) = document(sheet) else {
        out.push_str(
            "<p class=\"note\">The body of this message has not been downloaded, \
             so only its headers are printed.</p>\n",
        );
        return;
    };
    out.push_str("<div class=\"body\">\n");
    if let Some(action) = &document.primary {
        button(out, &action.label, action.url.as_str());
    }
    blocks(out, &document.blocks, sheet.remote);
    out.push_str("</div>\n");
    if document.reached != Reached::Nothing {
        out.push_str(
            "<p class=\"note\">This message is longer than can be shown; \
             the rest of it is not printed.</p>\n",
        );
    }
}

/// What a paginating renderer should not cut in two: see the module notes.
const KEEP: &str = " data-break-inside=\"avoid\"";

fn blocks(out: &mut String, list: &[Block], remote: Remote<'_>) {
    for block in list {
        one_block(out, block, remote);
    }
}

fn one_block(out: &mut String, block: &Block, remote: Remote<'_>) {
    match block {
        Block::Heading { level, spans } => {
            // Message headings sit under the message's own `h2`.
            let level = (*level).clamp(1, 6).saturating_add(2).min(6);
            let _ = write!(out, "<h{level}>");
            inline(out, spans);
            let _ = writeln!(out, "</h{level}>");
        }
        Block::Paragraph { spans, dir } => {
            let _ = write!(out, "<p dir=\"{}\">", direction(*dir));
            inline(out, spans);
            out.push_str("</p>\n");
        }
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let _ = writeln!(out, "<{tag}>");
            for item in items {
                out.push_str("<li>");
                blocks(out, item, remote);
                out.push_str("</li>\n");
            }
            let _ = writeln!(out, "</{tag}>");
        }
        Block::Quote {
            attribution,
            blocks: inner,
        } => {
            if let Some(spans) = attribution {
                out.push_str("<p class=\"attribution\">");
                inline(out, spans);
                out.push_str("</p>\n");
            }
            let _ = writeln!(out, "<blockquote{KEEP}>");
            blocks(out, inner, remote);
            out.push_str("</blockquote>\n");
        }
        Block::Code { text, .. } => {
            let _ = writeln!(out, "<pre>{}</pre>", escape(text));
        }
        Block::Table { head, rows } => {
            let _ = writeln!(out, "<table{KEEP}>");
            if let Some(head) = head {
                out.push_str("<thead><tr>");
                for cell in head {
                    out.push_str("<th>");
                    inline(out, cell);
                    out.push_str("</th>");
                }
                out.push_str("</tr></thead>\n");
            }
            out.push_str("<tbody>\n");
            for cells in rows {
                out.push_str("<tr>");
                for cell in cells {
                    out.push_str("<td>");
                    inline(out, cell);
                    out.push_str("</td>");
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody>\n</table>\n");
        }
        Block::Facts(pairs) => {
            out.push_str("<dl>\n");
            for (label, value) in pairs {
                out.push_str("<dt>");
                inline(out, label);
                out.push_str("</dt><dd>");
                inline(out, value);
                out.push_str("</dd>\n");
            }
            out.push_str("</dl>\n");
        }
        Block::Image {
            src,
            alt,
            width,
            height,
        } => image(out, src, alt, (*width, *height), remote),
        Block::Button { label, url } => button(out, label, url.as_str()),
        Block::Signature(inner) => {
            out.push_str("<div class=\"signature\">\n");
            blocks(out, inner, remote);
            out.push_str("</div>\n");
        }
        Block::Rule => out.push_str("<hr>\n"),
    }
}

fn direction(dir: Dir) -> &'static str {
    match dir {
        Dir::Auto => "auto",
        Dir::Ltr => "ltr",
        Dir::Rtl => "rtl",
    }
}

/// An image that is here is drawn; one that is not is named.
///
/// [`ImgSrc::Inline`] is drawn: a `data:` URI of an allowlisted image type that the block parser
/// built. A remote image is drawn only when the reader consented to this message's images and
/// the caller hands in its bytes, already fetched, as such a `data:` URI ([`Remote::Allowed`]);
/// otherwise it is named. Nothing here fetches: a printout that fetched would be a read receipt
/// nobody asked for.
fn image(
    out: &mut String,
    src: &ImgSrc,
    alt: &str,
    size: (Option<u32>, Option<u32>),
    remote: Remote<'_>,
) {
    let (width, height) = size;
    let alt = alt.trim();
    match src {
        ImgSrc::Inline(uri) => draw(out, uri.as_str(), alt, size),
        // A tracking pixel or a spacer: there was never anything to see.
        ImgSrc::Remote(_) | ImgSrc::Blocked { .. } if is_spacer(width, height) => {}
        ImgSrc::Remote(url) => {
            // The consented image's bytes, when the caller fetched them and they are an image.
            let fetched = match remote {
                Remote::Allowed(fetched) => fetched
                    .get(url.as_str())
                    .map(String::as_str)
                    .filter(|uri| drawable(uri)),
                Remote::Blocked => None,
            };
            match fetched {
                Some(uri) => draw(out, uri, alt, size),
                None => {
                    let host = url::Url::parse(url.as_str())
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_owned));
                    missing(out, alt, host.as_deref());
                }
            }
        }
        ImgSrc::Blocked { host } => missing(out, alt, Some(host)),
    }
}

/// An `<img>` of `uri`, a `data:` URI this module has checked, on a line of its own that a page's
/// end does not cut.
fn draw(out: &mut String, uri: &str, alt: &str, (width, height): (Option<u32>, Option<u32>)) {
    let _ = write!(
        out,
        "<p{KEEP}><img src=\"{}\" alt=\"{}\"",
        escape(uri),
        escape(alt)
    );
    if let Some(width) = width {
        let _ = write!(out, " width=\"{width}\"");
    }
    if let Some(height) = height {
        let _ = write!(out, " height=\"{height}\"");
    }
    out.push_str("></p>\n");
}

/// A declared side of one pixel or less.
fn is_spacer(width: Option<u32>, height: Option<u32>) -> bool {
    width.is_some_and(|w| w <= 1) || height.is_some_and(|h| h <= 1)
}

fn missing(out: &mut String, alt: &str, host: Option<&str>) {
    let what = if alt.is_empty() {
        "[image".to_owned()
    } else {
        format!("[image: {alt}")
    };
    let from = host
        .map(|host| format!(", from {host}"))
        .unwrap_or_default();
    let _ = writeln!(out, "{MISSING}{}{}]</p>", escape(&what), escape(&from));
}

/// A lone link. On paper a link cannot be followed, so its address is printed beside it.
fn button(out: &mut String, label: &str, url: &str) {
    let _ = writeln!(
        out,
        "<p class=\"button\"><a href=\"{}\">{}</a> <span class=\"url\">&lt;{}&gt;</span></p>",
        escape(url),
        escape(label),
        escape(url)
    );
}

fn inline(out: &mut String, spans: &[Span]) {
    for span in spans {
        match span {
            Span::Text(text) => out.push_str(&escape(text)),
            Span::Strong(inner) => {
                out.push_str("<strong>");
                inline(out, inner);
                out.push_str("</strong>");
            }
            Span::Emphasis(inner) => {
                out.push_str("<em>");
                inline(out, inner);
                out.push_str("</em>");
            }
            Span::Code(text) => {
                let _ = write!(out, "<code>{}</code>", escape(text));
            }
            Span::Link { url, spans } => {
                let _ = write!(out, "<a href=\"{}\">", escape(url.as_str()));
                inline(out, spans);
                out.push_str("</a>");
            }
            Span::Break => out.push_str("<br>"),
        }
    }
}

fn attachments(out: &mut String, message: &Message) {
    let listed: Vec<_> = message
        .attachments
        .iter()
        .filter(|part| part.inline == mail_domain::Inline::Attached)
        .collect();
    if listed.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "<section class=\"attachments\"{KEEP}>\n<strong>Attachments ({})</strong>\n<ul>",
        listed.len()
    );
    for part in listed {
        let name = part.name.trim();
        let name = if name.is_empty() { "(unnamed)" } else { name };
        let _ = writeln!(
            out,
            "<li>{} <span class=\"size\">({})</span></li>",
            escape(name),
            escape(&size(part.size))
        );
    }
    out.push_str("</ul>\n</section>\n");
}

/// A size a person can read, in the units `mailo attachments` uses.
fn size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    match bytes {
        0..KB => format!("{bytes} B"),
        KB..MB => format!("{:.1} kB", bytes as f64 / KB as f64),
        _ => format!("{:.1} MB", bytes as f64 / MB as f64),
    }
}

/// Text made safe for both element content and a double-quoted attribute.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            // A NUL or other control character has no business in a printout and some
            // renderers treat NUL specially; newlines and tabs are whitespace and stay.
            '\n' | '\t' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
