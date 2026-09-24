//! HTML tokens to blocks.
//!
//! The input is ammonia's serializer output: balanced tags, escaped text, the
//! allowlist. A tokenizer plus this stack is enough, and the caps apply as
//! tokens arrive rather than after a tree is allocated. `process_token` takes
//! `&self` in html5ever 0.40, so the stack lives in a [`RefCell`].
//!
//! An unknown tag is transparent. On this input an unrecognised tag is one
//! ammonia's allowlist grew, not one an attacker chose; dropping it would lose
//! mail the day ammonia adds `main`. End tags that do not match are ignored.

mod drive;
mod emit;
mod tags;

pub use tags::{is_mapped, mapped_tags};

use self::tags::{Frame, Kind, flush_buf, frame_kind, is_void, presentational, push_chars};
use super::heaviness::Signals;
use super::kind::{Block, first_strong, span_text, strip_overrides};
use super::limits::{Limits, Reached};
use crate::parse::ParsedPart;
use crate::sanitize::RemoteImages;
use html5ever::buffer_queue::BufferQueue;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    Tag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use std::cell::RefCell;

pub(crate) struct HtmlOut {
    pub blocks: Vec<Block>,
    pub reached: Reached,
    pub signals: Signals,
}

/// What the walk does with a `cid:` image whose bytes it cannot embed.
///
/// The reader drops it: an image that is not there has nothing to draw, and the parts list
/// already names the file. A printout has no parts pane beside it, so it says in the text that
/// an image stood there, by its own description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unheld {
    Drop,
    Describe,
}

pub(crate) fn walk(
    html: &str,
    parts: &[ParsedPart],
    images: RemoteImages,
    unheld: Unheld,
) -> HtmlOut {
    let mut tendril = StrTendril::new();
    tendril.push_slice(html);
    let queue = BufferQueue::default();
    queue.push_back(tendril);
    let sink = Sink {
        inner: RefCell::new(Builder::new(parts, images, unheld)),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    tokenizer.sink.inner.borrow_mut().finish()
}

struct Sink<'a> {
    inner: RefCell<Builder<'a>>,
}

impl TokenSink for Sink<'_> {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        self.inner.borrow_mut().token(token);
        TokenSinkResult::Continue
    }
}

struct Builder<'a> {
    stack: Vec<Frame>,
    /// Depth overflow: tags stay transparent and their text is kept.
    skip_keep: u32,
    /// Node overflow: further tags and text are dropped.
    skip_drop: u32,
    nodes: u32,
    text_bytes: usize,
    block_count: usize,
    long_run: u32,
    reached: Reached,
    signals: Signals,
    parts: &'a [ParsedPart],
    images: RemoteImages,
    unheld: Unheld,
    spent: usize,
    closed: bool,
    output: Vec<Block>,
}

impl<'a> Builder<'a> {
    fn new(parts: &'a [ParsedPart], images: RemoteImages, unheld: Unheld) -> Self {
        Self {
            stack: vec![Frame::root()],
            skip_keep: 0,
            skip_drop: 0,
            nodes: 0,
            text_bytes: 0,
            block_count: 0,
            long_run: 0,
            reached: Reached::Nothing,
            signals: Signals::default(),
            parts,
            images,
            unheld,
            spent: 0,
            closed: false,
            output: Vec::new(),
        }
    }

    fn finish(&mut self) -> HtmlOut {
        self.close_all();
        HtmlOut {
            blocks: std::mem::take(&mut self.output),
            reached: self.reached,
            signals: std::mem::take(&mut self.signals),
        }
    }

    fn token(&mut self, token: Token) {
        match token {
            TagToken(tag) => self.on_tag(tag),
            Token::CharacterTokens(text) => self.on_text(&text),
            Token::NullCharacterToken => {}
            Token::EOFToken => self.close_all(),
            Token::CommentToken(_) | Token::DoctypeToken(_) | Token::ParseError(_) => {}
        }
    }

    fn on_tag(&mut self, tag: Tag) {
        let name: &str = &tag.name;
        let start = matches!(tag.kind, html5ever::tokenizer::StartTag);
        if self.skip_drop > 0 {
            if start && !is_void(name) {
                self.skip_drop += 1;
            } else if !start && self.skip_drop > 0 {
                self.skip_drop -= 1;
            }
            return;
        }
        if self.skip_keep > 0 {
            if start && is_void(name) {
                self.void_tag(&tag);
            } else if start {
                self.skip_keep += 1;
            } else {
                self.skip_keep -= 1;
            }
            return;
        }
        if !start {
            if matches!(name, "thead" | "tbody" | "tfoot" | "colgroup") {
                self.touch_table(|table| table.in_head = false);
                return;
            }
            self.close_name(name);
            return;
        }
        if self.nodes >= Limits::MAX_NODES {
            self.note(Reached::Nodes);
            if !is_void(name) {
                self.skip_drop += 1;
            }
            return;
        }
        self.nodes += 1;
        self.signals.elements += 1;
        if presentational(&tag) {
            self.signals.presentational += 1;
        }
        if matches!(name, "thead" | "tbody" | "tfoot" | "colgroup") {
            self.touch_table(|table| table.in_head = name == "thead");
            return;
        }
        if self.stack.len() >= Limits::MAX_DEPTH {
            self.note(Reached::Depth);
            if is_void(name) {
                self.void_tag(&tag);
            } else {
                self.skip_keep += 1;
            }
            return;
        }
        if is_void(name) {
            self.void_tag(&tag);
            return;
        }
        let Some(kind) = frame_kind(name) else {
            // Unknown: transparent. Children attach to the parent.
            return;
        };
        self.open(kind, name, &tag);
        if tag.self_closing {
            self.finish_top();
        }
    }

    fn on_text(&mut self, text: &str) {
        if self.skip_drop > 0 || self.closed {
            return;
        }
        let clean = strip_overrides(text);
        if clean.is_empty() {
            return;
        }
        let raw = self.stack.last().is_some_and(|frame| frame.raw);
        let mut buf = String::new();
        let mut sticky = false;
        if let Some(frame) = self.stack.last_mut() {
            sticky = !frame.spans.is_empty();
            buf = std::mem::take(&mut frame.buf);
        }
        let before = buf.len();
        push_chars(
            &mut buf,
            &clean,
            raw,
            sticky,
            &mut self.text_bytes,
            &mut self.reached,
        );
        let added = buf.len() - before;
        if added > 0 {
            self.signals.text_chars += clean.chars().count() as u32;
        }
        if let Some(frame) = self.stack.last_mut() {
            frame.buf = buf;
        }
    }

    fn close_name(&mut self, name: &str) {
        let Some(index) = self.stack.iter().rposition(|frame| frame.name == name) else {
            return;
        };
        while self.stack.len() > index + 1 {
            self.finish_top();
        }
        self.finish_top();
    }

    fn finish_top(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        let produced = self.reduce(frame);
        self.attach(produced);
    }

    fn close_all(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        while self.stack.len() > 1 {
            self.finish_top();
        }
        if let Some(mut root) = self.stack.pop() {
            flush_buf(&mut root);
            let spans = std::mem::take(&mut root.spans);
            if !span_text(&spans).trim().is_empty() {
                let dir = first_strong(&span_text(&spans));
                if self.block_count < Limits::MAX_BLOCKS {
                    self.block_count += 1;
                    root.blocks.push(Block::Paragraph { spans, dir });
                } else {
                    self.note(Reached::Blocks);
                }
            }
            self.output = root.blocks;
        }
    }

    fn touch_table(&mut self, f: impl FnOnce(&mut Frame)) {
        if let Some(table) = self
            .stack
            .iter_mut()
            .rev()
            .find(|frame| frame.kind == Kind::Table)
        {
            f(table);
        }
    }

    fn note(&mut self, next: Reached) {
        if self.reached == Reached::Nothing {
            self.reached = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Unheld, walk};
    use crate::block::Block;
    use crate::block::kind::span_text;
    use crate::sanitize::RemoteImages;

    #[test]
    fn an_unknown_tag_keeps_its_subtree() {
        // Ammonia unwraps tags it does not allow, so this calls the walker on
        // the markup ammonia would emit the day it allows `main`.
        let out = walk(
            "<main><p>kept</p>inner</main>",
            &[],
            RemoteImages::Blocked,
            Unheld::Drop,
        );
        let mut text = String::new();
        for block in &out.blocks {
            if let Block::Paragraph { spans, .. } = block {
                text.push_str(&span_text(spans));
            }
        }
        assert!(
            text.contains("kept") && text.contains("inner"),
            "unknown tag dropped its subtree: {text}"
        );
    }
}
