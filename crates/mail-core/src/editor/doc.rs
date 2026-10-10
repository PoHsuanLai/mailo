//! The composer's document: one column of paragraphs and atomic objects.
//!
//! Offsets are grapheme clusters. A backspace deletes what the reader sees as one
//! character, including an emoji family joined by ZWJ.

use crate::editor::text::normalize_runs;
use mail_mime::SafeUrl;
use mail_mime::block::Block;

/// A heading's level. One, two, or three: deeper headings are not a thing this editor writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// `# `.
    One,
    /// `## `.
    Two,
    /// `### `.
    Three,
}

impl Level {
    /// `1`, `2`, or `3`.
    pub fn number(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
        }
    }

    /// `Some` for 1, 2, or 3.
    pub fn from_number(level: u8) -> Option<Self> {
        match level {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            _ => None,
        }
    }
}

/// Whether a to-do box is ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// An empty box.
    Open,
    /// A ticked box.
    Done,
}

/// What a paragraph is. Objects are not kinds: they are [`Object`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParaKind {
    /// Body text.
    Paragraph,
    /// A section heading.
    Heading(Level),
    /// One item of a bulleted list.
    Bullet,
    /// One item of a numbered list. The number is the item's place in a run, not stored here.
    Numbered,
    /// One checkable item.
    Todo(Check),
    /// A quoted passage.
    Quote,
    /// Preformatted text. Newlines inside it are the author's.
    Code,
}

/// Which inline mark [`super::op::Op::SetMark`] talks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Bold.
    Bold,
    /// Italic.
    Italic,
    /// Underline.
    Underline,
    /// Strikethrough.
    Strike,
    /// Inline code.
    Code,
}

/// Whether a mark is applied or cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// The mark covers the range.
    On,
    /// The mark does not cover the range.
    Off,
}

/// Inline marks on one run, plus an optional link.
///
/// The five styles are bits. The link is its own field because a URL is not a flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marks {
    bits: u8,
    /// Where this run links, when it links.
    pub link: Option<SafeUrl>,
}

const BIT_BOLD: u8 = 1 << 0;
const BIT_ITALIC: u8 = 1 << 1;
const BIT_UNDERLINE: u8 = 1 << 2;
const BIT_STRIKE: u8 = 1 << 3;
const BIT_CODE: u8 = 1 << 4;

impl Marks {
    /// No marks and no link.
    pub fn new() -> Self {
        Self {
            bits: 0,
            link: None,
        }
    }

    /// Whether `mark` is set.
    pub fn has(&self, mark: Mark) -> bool {
        self.bits & bit(mark) != 0
    }

    /// Set or clear `mark`. The link is untouched.
    pub fn set(&mut self, mark: Mark, on: Presence) {
        match on {
            Presence::On => self.bits |= bit(mark),
            Presence::Off => self.bits &= !bit(mark),
        }
    }

    /// `On` when `mark` is set.
    pub fn presence(&self, mark: Mark) -> Presence {
        if self.has(mark) {
            Presence::On
        } else {
            Presence::Off
        }
    }
}

impl Default for Marks {
    fn default() -> Self {
        Self::new()
    }
}

fn bit(mark: Mark) -> u8 {
    match mark {
        Mark::Bold => BIT_BOLD,
        Mark::Italic => BIT_ITALIC,
        Mark::Underline => BIT_UNDERLINE,
        Mark::Strike => BIT_STRIKE,
        Mark::Code => BIT_CODE,
    }
}

/// A stretch of text that shares one [`Marks`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The text, which may contain `\n` for a shift-enter line break.
    pub text: String,
    /// The marks on `text`.
    pub marks: Marks,
}

impl Run {
    /// One run.
    pub fn new(text: impl Into<String>, marks: Marks) -> Self {
        Self {
            text: text.into(),
            marks,
        }
    }
}

/// An image the composer embeds. The id is the app's, not a URL the reader fetches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef(String);

impl ImageRef {
    /// An image id the app can resolve.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A file attached to the message, named for the chip and the plain-text line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef(String);

impl AttachmentRef {
    /// The file name the reader sees.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The file name.
    pub fn name(&self) -> &str {
        &self.0
    }
}

/// A simple table. Cells are text; a cell is not a nested document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Row-major cells.
    pub rows: Vec<Vec<String>>,
}

/// Something that is not a paragraph. The caret does not go inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Object {
    /// An embedded image and its alt text.
    Image {
        /// Where the pixels live.
        src: ImageRef,
        /// The alt text.
        alt: String,
    },
    /// A data table.
    Table(Table),
    /// A horizontal rule.
    Divider,
    /// A file on the message. The bytes live in the draft, not in the document.
    Attachment(AttachmentRef),
    /// The `-- ` signature separator. Lines after it are the signature body.
    Signature,
    /// The folded original on a reply.
    QuotedMessage {
        /// Who wrote it.
        who: String,
        /// When, as the composer shows it.
        when: String,
        /// The quoted blocks.
        body: Vec<Block>,
    },
}

/// One node of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A paragraph, heading, list item, quote, or code block.
    Para {
        /// Which of those it is.
        kind: ParaKind,
        /// Its text, split where the marks change.
        runs: Vec<Run>,
    },
    /// An atomic object.
    Object(Object),
}

impl Node {
    /// A paragraph of `kind` whose runs are normalized.
    pub fn para(kind: ParaKind, runs: Vec<Run>) -> Self {
        let mut runs = runs;
        normalize_runs(&mut runs);
        Self::Para { kind, runs }
    }

    /// A paragraph of `kind` holding `text` with no marks.
    pub fn plain(kind: ParaKind, text: impl Into<String>) -> Self {
        Self::para(kind, vec![Run::new(text, Marks::new())])
    }
}

/// The message body being written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    /// Top to bottom.
    pub nodes: Vec<Node>,
}

impl Doc {
    /// One empty paragraph, which is where a new message starts.
    pub fn blank() -> Self {
        Self {
            nodes: vec![Node::plain(ParaKind::Paragraph, "")],
        }
    }

    /// One paragraph holding `text`.
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            nodes: vec![Node::plain(ParaKind::Paragraph, text)],
        }
    }
}

impl Default for Doc {
    fn default() -> Self {
        Self::blank()
    }
}

/// A caret position. `offset` counts grapheme clusters from the start of `node`.
///
/// An object has length 1: offset 0 is before it, offset 1 is after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    /// Index into [`Doc::nodes`].
    pub node: usize,
    /// Grapheme clusters into that node.
    pub offset: usize,
}

impl Pos {
    /// A position.
    pub fn new(node: usize, offset: usize) -> Self {
        Self { node, offset }
    }
}

/// A range of the document, in grapheme positions. Ends are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// Where the range starts.
    pub start: Pos,
    /// Where the range ends, exclusive.
    pub end: Pos,
}

impl Range {
    /// A range, reordered so `start` is not after `end`.
    pub fn ordered(self) -> Self {
        if (self.end.node, self.end.offset) < (self.start.node, self.start.offset) {
            Self {
                start: self.end,
                end: self.start,
            }
        } else {
            self
        }
    }

    /// The two ends are the same position.
    pub fn is_collapsed(self) -> bool {
        let range = self.ordered();
        range.start == range.end
    }
}
