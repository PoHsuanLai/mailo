//! Opening a tag, and the blocks a void tag is by itself.

use super::Builder;
use super::tags::{Frame, Kind, attr, code_lang, dir_attr, flush_buf, parse_px};
use crate::block::image::{is_spacer, resolve};
use crate::block::kind::{Block, Dir, Span, first_strong, span_text, strip_overrides};
use crate::block::limits::Limits;
use crate::block::limits::Reached;
use crate::block::url::SafeUrl;
use html5ever::tokenizer::Tag;

impl<'a> Builder<'a> {
    pub(super) fn open(&mut self, kind: Kind, name: &str, tag: &Tag) {
        // Inside `<pre>`, inline tags must not steal the text. `code`'s `lang`
        // is the one attribute ammonia still lets through that names a language.
        if self.stack.last().is_some_and(|frame| frame.raw) && kind.is_inline() {
            if kind == Kind::Code
                && let Some(lang) = attr(tag, "lang").and_then(code_lang)
            {
                for frame in self.stack.iter_mut().rev() {
                    if frame.kind == Kind::Pre && frame.lang.is_none() {
                        frame.lang = Some(lang);
                        break;
                    }
                }
            }
            return;
        }
        if kind.is_block() {
            self.prepare_block();
        }
        if kind == Kind::Table && self.stack.iter().any(|frame| frame.kind == Kind::Table) {
            self.signals.tables_in_tables += 1;
            for frame in &mut self.stack {
                if frame.kind == Kind::Table {
                    frame.layout = true;
                }
            }
        }
        if matches!(
            kind,
            Kind::Blocks
                | Kind::Paragraph
                | Kind::Heading(_)
                | Kind::Quote
                | Kind::List
                | Kind::Pre
                | Kind::Table
        ) {
            self.mark_table_layout();
        }
        if matches!(name, "td" | "th") {
            self.signals.table_cells += 1;
        }
        let forced = match dir_attr(tag) {
            Some(dir @ (Dir::Ltr | Dir::Rtl)) => Some(dir),
            _ => self.stack.last().and_then(|frame| frame.forced),
        };
        if name == "bdo"
            && let Some(dir) = forced
        {
            for frame in self.stack.iter_mut().rev() {
                if matches!(frame.kind, Kind::Paragraph | Kind::Heading(_))
                    && frame.forced.is_none()
                {
                    frame.forced = Some(dir);
                    break;
                }
            }
        }
        let mut frame = Frame::new(kind, name, forced);
        frame.ordered = name == "ol";
        frame.header = name == "th";
        frame.raw = kind == Kind::Pre;
        if kind == Kind::Anchor {
            frame.link = attr(tag, "href").and_then(SafeUrl::parse);
            if frame.link.is_some() {
                self.signals.links += 1;
            }
        }
        if kind == Kind::Pre {
            frame.lang = attr(tag, "lang").and_then(code_lang);
        }
        self.signals.max_depth = self.signals.max_depth.max(self.stack.len() as u32 + 1);
        self.flush_stray();
        self.stack.push(frame);
    }

    pub(super) fn void_tag(&mut self, tag: &Tag) {
        let name: &str = &tag.name;
        match name {
            "br" => self.push_break(),
            "hr" => {
                self.prepare_block();
                self.mark_table_layout();
                self.flush_stray();
                self.emit_here(Block::Rule, false);
            }
            "img" => {
                let width = attr(tag, "width").and_then(parse_px);
                let height = attr(tag, "height").and_then(parse_px);
                if is_spacer(width, height) {
                    self.signals.spacer_images += 1;
                }
                let Some(src) = attr(tag, "src") else {
                    return;
                };
                let Some(resolved) = resolve(src, self.parts, self.images, &mut self.spent) else {
                    return;
                };
                self.prepare_block();
                self.mark_table_layout();
                self.flush_stray();
                let alt = attr(tag, "alt").unwrap_or("");
                self.emit_here(
                    Block::Image {
                        src: resolved,
                        alt: strip_overrides(alt),
                        width,
                        height,
                    },
                    false,
                );
            }
            _ => {}
        }
    }

    pub(super) fn push_break(&mut self) {
        let Some(frame) = self.stack.last_mut() else {
            return;
        };
        if frame.raw {
            if frame.buf.len() < Limits::MAX_RUN {
                frame.buf.push('\n');
            }
            return;
        }
        let text = std::mem::take(&mut frame.buf);
        if !text.is_empty() {
            frame.spans.push(Span::Text(text));
        }
        frame.spans.push(Span::Break);
    }

    pub(super) fn prepare_block(&mut self) {
        while self
            .stack
            .last()
            .is_some_and(|frame| frame.kind.closes_when_block_opens())
        {
            self.finish_top();
        }
    }

    pub(super) fn mark_table_layout(&mut self) {
        if let Some(table) = self
            .stack
            .iter_mut()
            .rev()
            .find(|frame| frame.kind == Kind::Table)
        {
            table.layout = true;
        }
    }

    pub(super) fn flush_stray(&mut self) {
        let accepts = self
            .stack
            .last()
            .is_some_and(|frame| frame.kind.accepts_blocks());
        if !accepts {
            return;
        }
        let (spans, forced) = {
            let Some(parent) = self.stack.last_mut() else {
                return;
            };
            flush_buf(parent);
            (std::mem::take(&mut parent.spans), parent.forced)
        };
        if span_text(&spans).trim().is_empty() {
            return;
        }
        let dir = forced.unwrap_or_else(|| first_strong(&span_text(&spans)));
        self.emit_here(Block::Paragraph { spans, dir }, false);
    }

    pub(super) fn emit_here(&mut self, block: Block, from_p: bool) {
        self.note_shape(&block, from_p);
        if self.block_count >= Limits::MAX_BLOCKS {
            self.note(Reached::Blocks);
            return;
        }
        self.block_count += 1;
        if let Some(parent) = self.stack.last_mut() {
            parent.blocks.push(block);
        }
    }

    pub(super) fn note_shape(&mut self, block: &Block, from_p: bool) {
        if from_p {
            let long = match block {
                Block::Paragraph { spans, .. } => span_text(spans).chars().count() >= 80,
                _ => false,
            };
            if long {
                self.long_run += 1;
                self.signals.long_paragraphs = self.signals.long_paragraphs.max(self.long_run);
                return;
            }
        }
        self.long_run = 0;
    }
}
