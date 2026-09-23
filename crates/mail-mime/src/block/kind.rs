//! The blocks a message body lowers to.
//!
//! Nothing here derives serde. A serde form is a persisted schema, and render
//! output must not be persisted: a cap or a mapping change would otherwise
//! freeze yesterday's blocks in the database. Re-parse the message.

use super::limits::Reached;
use super::url::SafeUrl;

/// How heavily the body is laid out, and whether it is a machine talking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Prose. A letter, a reply, a note.
    Letter,
    /// Tables, spacers, presentational markup. The blocks open; the sandboxed
    /// original is the other view.
    Layout,
    /// A layout-heavy body that is also a receipt, a tracking mail, a
    /// notification: one action, then facts.
    Machine,
}

/// The one thing a [`Shape::Machine`] body is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// The link's own words, whitespace collapsed.
    pub label: String,
    /// Where the action goes.
    pub url: SafeUrl,
}

/// Base direction of a paragraph or heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// No strong character and no `dir` attribute.
    Auto,
    /// Left to right.
    Ltr,
    /// Right to left.
    Rtl,
}

/// Whether plain text may be rewrapped, and whether the flow space is real.
///
/// `delsp` is meaningless for [`Flowed::Fixed`], so it is not a separate flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flowed {
    /// `format=fixed`. Lines are the sender's line breaks.
    Fixed,
    /// `format=flowed`. A trailing space joins the next line, except on `-- `.
    Flowed {
        /// When set, the trailing space is a marker and is deleted on join.
        /// When clear, the space is a real space that also marks the join.
        delsp: bool,
    },
}

/// One block in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `h1`–`h6`. `level` is 1 through 6.
    Heading { level: u8, spans: Vec<Span> },
    /// A paragraph. `dir` comes from the `dir` attribute, else the first strong character.
    Paragraph { spans: Vec<Span>, dir: Dir },
    /// A list. Items are blocks, so a list can hold a paragraph, a quote, or another list.
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    /// A quotation. `attribution` is the line that introduced it, when one did.
    Quote {
        attribution: Option<Vec<Span>>,
        blocks: Vec<Block>,
    },
    /// Preformatted text. `lang` is set only when the sender named one.
    Code { lang: Option<String>, text: String },
    /// A data table. Cells are inline; a table used for layout is flattened instead.
    Table {
        head: Option<Vec<Vec<Span>>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    /// Label/value pairs taken from a two-column table in machine mail.
    Facts(Vec<(Vec<Span>, Vec<Span>)>),
    /// An image, inline or remote or blocked.
    Image {
        src: ImgSrc,
        alt: String,
        width: Option<u32>,
        height: Option<u32>,
    },
    /// A short link that was the only thing in its paragraph.
    Button { label: String, url: SafeUrl },
    /// The signature after the last `-- ` at quote depth 0.
    Signature(Vec<Block>),
    /// A horizontal rule.
    Rule,
}

/// Inline content inside a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    /// Text with bidi overrides already removed.
    Text(String),
    /// `b` / `strong`.
    Strong(Vec<Span>),
    /// `i` / `em` and the other italic-shaped tags.
    Emphasis(Vec<Span>),
    /// Inline `code`.
    Code(String),
    /// A link. The renderer adds `target` and `rel`; they are not stored here.
    Link { url: SafeUrl, spans: Vec<Span> },
    /// `br`.
    Break,
}

/// Where an image's pixels come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImgSrc {
    /// A `data:` URI built from this message's own part.
    Inline(Inlined),
    /// A remote image the reader has allowed.
    Remote(SafeUrl),
    /// A remote image the reader has not allowed.
    ///
    /// The host is the only thing kept. The URL is not: storing it would leave
    /// a fetchable address in the document the reader holds, which is the
    /// request the block was supposed to not make. The host is what the
    /// placeholder names, so the person can see who would learn they opened
    /// the mail before they choose to load it.
    Blocked { host: String },
}

/// A `data:image/...;base64,...` URI.
///
/// Built only from a part whose declared type [`crate::embeddable`] accepts.
/// The string is private so a caller cannot assemble `data:text/html` and hand
/// it to the renderer as if this crate had checked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inlined(String);

impl Inlined {
    pub(crate) fn new(uri: String) -> Self {
        Self(uri)
    }

    /// The `data:` URI.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A message body as blocks.
///
/// No serde derive, on purpose. See the module comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The body, in reading order.
    pub blocks: Vec<Block>,
    /// Letter, layout, or machine.
    pub shape: Shape,
    /// Which cap fired, if one did.
    pub reached: Reached,
    /// The action a [`Shape::Machine`] body was reduced to.
    pub primary: Option<Action>,
    bytes: usize,
    heaviness: i32,
}

impl Document {
    pub(crate) fn new(
        blocks: Vec<Block>,
        shape: Shape,
        reached: Reached,
        primary: Option<Action>,
        heaviness: i32,
    ) -> Self {
        let bytes = weight(&blocks, &primary);
        Self {
            blocks,
            shape,
            reached,
            primary,
            bytes,
            heaviness,
        }
    }

    /// Bytes of text and URLs the builder stored.
    ///
    /// Counted once, here, so a cache can weigh the document without walking
    /// it again on every insert and every eviction.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The heaviness score. Tests compare these; they do not pin the number.
    pub fn heaviness(&self) -> i32 {
        self.heaviness
    }
}

/// Plain text of `spans`, with breaks as spaces.
pub(crate) fn span_text(spans: &[Span]) -> String {
    let mut out = String::new();
    let mut stack: Vec<(&[Span], usize)> = vec![(spans, 0)];
    while let Some((level, index)) = stack.last_mut() {
        if *index >= level.len() {
            stack.pop();
            continue;
        }
        let span = &level[*index];
        *index += 1;
        match span {
            Span::Text(text) | Span::Code(text) => out.push_str(text),
            Span::Break => out.push(' '),
            Span::Strong(inner) | Span::Emphasis(inner) | Span::Link { spans: inner, .. } => {
                stack.push((inner, 0));
            }
        }
    }
    out
}

/// Drop bidi overrides. LRM and RLM stay: Hebrew and Arabic mail uses them.
pub(crate) fn strip_overrides(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if !matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') {
            out.push(ch);
        }
    }
    out
}

/// Direction from the first strong character. Neutrals and weak marks are skipped.
pub(crate) fn first_strong(text: &str) -> Dir {
    for ch in text.chars() {
        if is_rtl_letter(ch) {
            return Dir::Rtl;
        }
        if ch.is_alphabetic() {
            return Dir::Ltr;
        }
    }
    Dir::Auto
}

fn is_rtl_letter(ch: char) -> bool {
    matches!(
        ch,
        '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFE}'
    ) && ch.is_alphabetic()
}

pub(crate) fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn weight(blocks: &[Block], primary: &Option<Action>) -> usize {
    let mut total = 0usize;
    if let Some(action) = primary {
        total += action.label.len() + action.url.as_str().len();
    }
    let mut stack: Vec<&[Block]> = vec![blocks];
    let mut index: Vec<usize> = vec![0];
    while let Some(cursor) = index.last_mut() {
        let level = match stack.last() {
            Some(level) => *level,
            None => break,
        };
        if *cursor >= level.len() {
            stack.pop();
            index.pop();
            continue;
        }
        let block = &level[*cursor];
        *cursor += 1;
        match block {
            Block::Heading { spans, .. } | Block::Paragraph { spans, .. } => {
                total += span_weight(spans);
            }
            Block::List { items, .. } => {
                for item in items {
                    stack.push(item);
                    index.push(0);
                }
            }
            Block::Quote {
                attribution,
                blocks,
            } => {
                if let Some(spans) = attribution {
                    total += span_weight(spans);
                }
                stack.push(blocks);
                index.push(0);
            }
            Block::Code { lang, text } => {
                total += text.len() + lang.as_ref().map(String::len).unwrap_or(0);
            }
            Block::Table { head, rows } => {
                if let Some(head) = head {
                    for cell in head {
                        total += span_weight(cell);
                    }
                }
                for row in rows {
                    for cell in row {
                        total += span_weight(cell);
                    }
                }
            }
            Block::Facts(pairs) => {
                for (label, value) in pairs {
                    total += span_weight(label) + span_weight(value);
                }
            }
            Block::Image { src, alt, .. } => {
                total += alt.len();
                total += match src {
                    ImgSrc::Inline(uri) => uri.as_str().len(),
                    ImgSrc::Remote(url) => url.as_str().len(),
                    ImgSrc::Blocked { host } => host.len(),
                };
            }
            Block::Button { label, url } => {
                total += label.len() + url.as_str().len();
            }
            Block::Signature(blocks) => {
                stack.push(blocks);
                index.push(0);
            }
            Block::Rule => {}
        }
    }
    total
}

fn span_weight(spans: &[Span]) -> usize {
    let mut total = 0usize;
    let mut stack: Vec<(&[Span], usize)> = vec![(spans, 0)];
    while let Some((level, index)) = stack.last_mut() {
        if *index >= level.len() {
            stack.pop();
            continue;
        }
        let span = &level[*index];
        *index += 1;
        match span {
            Span::Text(text) | Span::Code(text) => total += text.len(),
            Span::Break => {}
            Span::Strong(inner) | Span::Emphasis(inner) => stack.push((inner, 0)),
            Span::Link { url, spans } => {
                total += url.as_str().len();
                stack.push((spans, 0));
            }
        }
    }
    total
}
