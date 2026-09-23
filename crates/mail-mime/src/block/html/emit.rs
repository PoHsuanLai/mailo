//! Closing a frame into blocks.

use super::Builder;
use super::tags::{Cell, Frame, Kind, Row, flush_buf, trim_spans};
use crate::block::kind::{Block, Span, first_strong, span_text};
use crate::block::limits::{Limits, Reached};

impl<'a> Builder<'a> {
    pub(super) fn reduce(&mut self, mut frame: Frame) -> Produced {
        match frame.kind {
            Kind::Strong | Kind::Emphasis | Kind::Code | Kind::Inline | Kind::Anchor => {
                flush_buf(&mut frame);
                let spans = std::mem::take(&mut frame.spans);
                if spans.is_empty() {
                    return Produced::Spans(Vec::new());
                }
                let wrapped = match frame.kind {
                    Kind::Strong => vec![Span::Strong(spans)],
                    Kind::Emphasis => vec![Span::Emphasis(spans)],
                    Kind::Code => vec![Span::Code(span_text(&spans))],
                    Kind::Anchor => match frame.link {
                        Some(url) => vec![Span::Link { url, spans }],
                        None => spans,
                    },
                    Kind::Inline => spans,
                    _ => spans,
                };
                Produced::Spans(wrapped)
            }
            Kind::Paragraph => self.paragraph(frame, true),
            Kind::Heading(level) => {
                flush_buf(&mut frame);
                trim_spans(&mut frame.spans);
                if frame.spans.is_empty() {
                    return Produced::Blocks {
                        blocks: Vec::new(),
                        from_p: false,
                    };
                }
                Produced::Blocks {
                    blocks: vec![Block::Heading {
                        level,
                        spans: frame.spans,
                    }],
                    from_p: false,
                }
            }
            Kind::Pre => {
                let mut text = frame.buf;
                if let Some(stripped) = text.strip_prefix('\n') {
                    text = stripped.to_owned();
                }
                if let Some(stripped) = text.strip_suffix('\n') {
                    text = stripped.to_owned();
                }
                if text.trim().is_empty() {
                    return Produced::Blocks {
                        blocks: Vec::new(),
                        from_p: false,
                    };
                }
                Produced::Blocks {
                    blocks: vec![Block::Code {
                        lang: frame.lang,
                        text,
                    }],
                    from_p: false,
                }
            }
            Kind::Blocks | Kind::Quote | Kind::List | Kind::Item | Kind::Cell => {
                self.fold_stray(&mut frame);
                match frame.kind {
                    Kind::Quote => Produced::Blocks {
                        blocks: vec![Block::Quote {
                            attribution: None,
                            blocks: frame.blocks,
                        }],
                        from_p: false,
                    },
                    Kind::List => {
                        let mut blocks = frame.blocks;
                        if !frame.items.is_empty() {
                            blocks.push(Block::List {
                                ordered: frame.ordered,
                                items: frame.items,
                            });
                        }
                        Produced::Blocks {
                            blocks,
                            from_p: false,
                        }
                    }
                    Kind::Item => Produced::Item(frame.blocks),
                    Kind::Cell => Produced::Cell(Cell {
                        blocks: frame.blocks,
                        header: frame.header,
                    }),
                    _ => Produced::Blocks {
                        blocks: frame.blocks,
                        from_p: false,
                    },
                }
            }
            Kind::Row => Produced::Row(frame.cells),
            Kind::Table => Produced::Blocks {
                blocks: reduce_table(frame),
                from_p: false,
            },
            Kind::Root => Produced::Blocks {
                blocks: frame.blocks,
                from_p: false,
            },
        }
    }

    pub(super) fn paragraph(&mut self, mut frame: Frame, from_p: bool) -> Produced {
        flush_buf(&mut frame);
        trim_spans(&mut frame.spans);
        if frame.spans.is_empty() {
            return Produced::Blocks {
                blocks: Vec::new(),
                from_p: false,
            };
        }
        let dir = frame
            .forced
            .unwrap_or_else(|| first_strong(&span_text(&frame.spans)));
        Produced::Blocks {
            blocks: vec![Block::Paragraph {
                spans: frame.spans,
                dir,
            }],
            from_p,
        }
    }

    pub(super) fn fold_stray(&mut self, frame: &mut Frame) {
        flush_buf(frame);
        trim_spans(&mut frame.spans);
        if frame.spans.is_empty() {
            return;
        }
        let dir = frame
            .forced
            .unwrap_or_else(|| first_strong(&span_text(&frame.spans)));
        frame.blocks.push(Block::Paragraph {
            spans: std::mem::take(&mut frame.spans),
            dir,
        });
    }

    pub(super) fn attach(&mut self, produced: Produced) {
        match produced {
            Produced::Spans(spans) => {
                if spans.is_empty() {
                    return;
                }
                let Some(parent) = self.stack.last_mut() else {
                    return;
                };
                let text = std::mem::take(&mut parent.buf);
                if !text.is_empty() {
                    parent.spans.push(Span::Text(text));
                }
                parent.spans.extend(spans);
            }
            Produced::Blocks { blocks, from_p } => {
                for block in blocks {
                    self.attach_block(block, from_p);
                }
            }
            Produced::Item(item) => {
                let in_list = self
                    .stack
                    .last()
                    .is_some_and(|frame| frame.kind == Kind::List);
                if !in_list {
                    for block in item {
                        self.attach_block(block, false);
                    }
                    return;
                }
                let over = self
                    .stack
                    .last()
                    .is_some_and(|frame| frame.items.len() >= Limits::MAX_ITEMS);
                if over {
                    self.note(Reached::Blocks);
                    return;
                }
                if let Some(parent) = self.stack.last_mut() {
                    parent.items.push(item);
                }
            }
            Produced::Cell(cell) => {
                let in_row = self
                    .stack
                    .last()
                    .is_some_and(|frame| frame.kind == Kind::Row);
                if !in_row {
                    for block in cell.blocks {
                        self.attach_block(block, false);
                    }
                    return;
                }
                let over = self
                    .stack
                    .last()
                    .is_some_and(|frame| frame.cells.len() >= Limits::MAX_COLS);
                if over {
                    self.note(Reached::Blocks);
                    return;
                }
                if let Some(parent) = self.stack.last_mut() {
                    parent.cells.push(cell);
                }
            }
            Produced::Row(cells) => {
                let in_table = self
                    .stack
                    .last()
                    .is_some_and(|frame| frame.kind == Kind::Table);
                if !in_table {
                    for cell in cells {
                        for block in cell.blocks {
                            self.attach_block(block, false);
                        }
                    }
                    return;
                }
                let head = self.stack.last().is_some_and(|frame| frame.in_head);
                let over = self
                    .stack
                    .last()
                    .is_some_and(|frame| frame.rows.len() >= Limits::MAX_ROWS);
                if over {
                    self.note(Reached::Blocks);
                    return;
                }
                if let Some(parent) = self.stack.last_mut() {
                    parent.rows.push(Row { cells, head });
                }
            }
        }
    }

    pub(super) fn attach_block(&mut self, block: Block, from_p: bool) {
        let block = self.with_attribution(block);
        self.note_shape(&block, from_p);
        if self.block_count >= Limits::MAX_BLOCKS {
            self.note(Reached::Blocks);
            return;
        }
        self.block_count += 1;
        if let Some(parent) = self.stack.last_mut() {
            parent.blocks.push(block);
        } else {
            self.output.push(block);
        }
    }

    pub(super) fn with_attribution(&mut self, block: Block) -> Block {
        let Block::Quote {
            blocks,
            attribution: None,
        } = block
        else {
            return block;
        };
        let Some(spans) = self.steal_attribution() else {
            return Block::Quote {
                attribution: None,
                blocks,
            };
        };
        Block::Quote {
            attribution: Some(spans),
            blocks,
        }
    }

    pub(super) fn steal_attribution(&mut self) -> Option<Vec<Span>> {
        let stolen = {
            let parent = self.stack.last_mut()?;
            let text = match parent.blocks.last() {
                Some(Block::Paragraph { spans, .. }) => span_text(spans),
                _ => return None,
            };
            if !crate::block::quote::is_attribution(&text) {
                return None;
            }
            match parent.blocks.pop() {
                Some(Block::Paragraph { spans, .. }) => Some(spans),
                Some(other) => {
                    parent.blocks.push(other);
                    None
                }
                None => None,
            }
        }?;
        self.block_count = self.block_count.saturating_sub(1);
        Some(stolen)
    }
}

pub(super) enum Produced {
    Spans(Vec<Span>),
    Blocks { blocks: Vec<Block>, from_p: bool },
    Item(Vec<Block>),
    Cell(Cell),
    Row(Vec<Cell>),
}

fn reduce_table(frame: Frame) -> Vec<Block> {
    let Frame {
        mut blocks,
        rows,
        layout,
        ..
    } = frame;
    let data = !layout
        && rows.iter().all(|row| {
            row.cells
                .iter()
                .all(|cell| matches!(cell.blocks.as_slice(), [] | [Block::Paragraph { .. }]))
        });
    if !data {
        for row in rows {
            for cell in row.cells {
                blocks.extend(cell.blocks);
            }
        }
        return blocks;
    }
    // `head` is one row. A later header row would be a second line of
    // labels; keep the first and let anything after it be body.
    let mut head: Option<Vec<Vec<Span>>> = None;
    let mut body = Vec::new();
    for row in rows {
        let span_row = row
            .cells
            .iter()
            .map(|cell| match cell.blocks.as_slice() {
                [Block::Paragraph { spans, .. }] => spans.clone(),
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        if span_row.is_empty() {
            continue;
        }
        let all_header = !row.cells.is_empty() && row.cells.iter().all(|cell| cell.header);
        if head.is_none() && (row.head || all_header) && body.is_empty() {
            head = Some(span_row);
        } else {
            body.push(span_row);
        }
    }
    if body.is_empty() && head.is_none() {
        return blocks;
    }
    blocks.push(Block::Table { head, rows: body });
    blocks
}
