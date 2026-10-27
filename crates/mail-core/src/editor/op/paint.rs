//! Setting an inline mark, a link, a size or a colour over a range, one paragraph at a time.

use mail_mime::{SafeUrl, TextColor, TextSize};

use super::{Op, check_range, covers, one_or_seq, paragraph_mut};
use crate::editor::doc::{Doc, Mark, Marks, Node, Pos, Presence, Range, Run};
use crate::editor::error::OpError;
use crate::editor::text::{grapheme_len, normalize_runs, para_len, split_at};

pub(super) fn set_mark(
    doc: &mut Doc,
    range: Range,
    mark: Mark,
    on: Presence,
) -> Result<Op, OpError> {
    let range = range.ordered();
    check_range(doc, range)?;
    let mut inverses = Vec::new();
    for index in 0..doc.nodes.len() {
        let Some((from, to)) = text_overlap(doc, index, range) else {
            continue;
        };
        let segments = {
            let Node::Para { runs, .. } = &doc.nodes[index] else {
                continue;
            };
            differing_marks(runs, from, to, mark, on)
        };
        let runs = paragraph_mut(doc, Pos::new(index, 0))?;
        paint_mark(runs, from, to, mark, on)?;
        for (start, end, previous) in segments {
            inverses.push(Op::SetMark {
                range: Range {
                    start: Pos::new(index, start),
                    end: Pos::new(index, end),
                },
                mark,
                on: previous,
            });
        }
    }
    Ok(one_or_seq(inverses, Op::SetMark { range, mark, on }))
}

pub(super) fn set_link(doc: &mut Doc, range: Range, url: Option<SafeUrl>) -> Result<Op, OpError> {
    set_field(
        doc,
        range,
        url,
        Field {
            get: |marks| marks.link.clone(),
            put: |marks, url| marks.link = url,
        },
        |range, url| Op::SetLink { range, url },
    )
}

pub(super) fn set_size(doc: &mut Doc, range: Range, size: Option<TextSize>) -> Result<Op, OpError> {
    set_field(
        doc,
        range,
        size,
        Field {
            get: |marks| marks.size,
            put: |marks, size| marks.size = size,
        },
        |range, size| Op::SetSize { range, size },
    )
}

pub(super) fn set_color(
    doc: &mut Doc,
    range: Range,
    color: Option<TextColor>,
) -> Result<Op, OpError> {
    set_field(
        doc,
        range,
        color,
        Field {
            get: |marks| marks.color,
            put: |marks, color| marks.color = color,
        },
        |range, color| Op::SetColor { range, color },
    )
}

/// One of a run's valued marks (its link, size or colour): how it is read and how it is written.
struct Field<T> {
    get: fn(&Marks) -> T,
    put: fn(&mut Marks, T),
}

/// Set a valued mark to `value` over `range`, and answer with the edits that put back what each
/// stretch had before, made by `op`.
fn set_field<T: Clone + PartialEq>(
    doc: &mut Doc,
    range: Range,
    value: T,
    field: Field<T>,
    op: fn(Range, T) -> Op,
) -> Result<Op, OpError> {
    let range = range.ordered();
    check_range(doc, range)?;
    let mut inverses = Vec::new();
    for index in 0..doc.nodes.len() {
        let Some((from, to)) = text_overlap(doc, index, range) else {
            continue;
        };
        let segments = {
            let Node::Para { runs, .. } = &doc.nodes[index] else {
                continue;
            };
            differing(runs, from, to, &value, &field)
        };
        let runs = paragraph_mut(doc, Pos::new(index, 0))?;
        paint(runs, from, to, &value, &field)?;
        for (start, end, previous) in segments {
            inverses.push(op(
                Range {
                    start: Pos::new(index, start),
                    end: Pos::new(index, end),
                },
                previous,
            ));
        }
    }
    Ok(one_or_seq(inverses, op(range, value)))
}

fn text_overlap(doc: &Doc, index: usize, range: Range) -> Option<(usize, usize)> {
    let len = match doc.nodes.get(index) {
        Some(Node::Para { runs, .. }) => para_len(runs),
        _ => return None,
    };
    if range.is_collapsed() || !covers(index, len, range) {
        return None;
    }
    let from = if range.start.node == index {
        range.start.offset
    } else {
        0
    };
    let to = if range.end.node == index {
        range.end.offset
    } else {
        len
    };
    if from >= to { None } else { Some((from, to)) }
}

fn differing_marks(
    runs: &[Run],
    from: usize,
    to: usize,
    mark: Mark,
    on: Presence,
) -> Vec<(usize, usize, Presence)> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    for run in runs {
        let end = seen + grapheme_len(&run.text);
        let start = from.max(seen);
        let stop = to.min(end);
        if start < stop && run.marks.presence(mark) != on {
            push_segment(&mut out, start, stop, run.marks.presence(mark));
        }
        seen = end;
    }
    out
}

/// The stretches of `[from, to)` whose value of `field` is not `value`, with the value each had.
fn differing<T: Clone + PartialEq>(
    runs: &[Run],
    from: usize,
    to: usize,
    value: &T,
    field: &Field<T>,
) -> Vec<(usize, usize, T)> {
    let mut out: Vec<(usize, usize, T)> = Vec::new();
    let mut seen = 0usize;
    for run in runs {
        let end = seen + grapheme_len(&run.text);
        let start = from.max(seen);
        let stop = to.min(end);
        let had = (field.get)(&run.marks);
        if start < stop && had != *value {
            let extend = out
                .last()
                .is_some_and(|(_, prev_end, previous)| *prev_end == start && *previous == had);
            if extend {
                if let Some((_, prev_end, _)) = out.last_mut() {
                    *prev_end = stop;
                }
            } else {
                out.push((start, stop, had));
            }
        }
        seen = end;
    }
    out
}

fn push_segment(
    out: &mut Vec<(usize, usize, Presence)>,
    start: usize,
    stop: usize,
    presence: Presence,
) {
    match out.last_mut() {
        Some((_, end, previous)) if *end == start && *previous == presence => *end = stop,
        _ => out.push((start, stop, presence)),
    }
}

fn paint_mark(
    runs: &mut Vec<Run>,
    from: usize,
    to: usize,
    mark: Mark,
    on: Presence,
) -> Result<(), OpError> {
    if from == to {
        return Ok(());
    }
    split_at(runs, to)?;
    let mut index = split_at(runs, from)?;
    let mut left = to - from;
    while left > 0 {
        let len = grapheme_len(&runs[index].text);
        runs[index].marks.set(mark, on);
        left -= len;
        index += 1;
    }
    normalize_runs(runs);
    Ok(())
}

fn paint<T: Clone>(
    runs: &mut Vec<Run>,
    from: usize,
    to: usize,
    value: &T,
    field: &Field<T>,
) -> Result<(), OpError> {
    if from == to {
        return Ok(());
    }
    split_at(runs, to)?;
    let mut index = split_at(runs, from)?;
    let mut left = to - from;
    while left > 0 {
        let len = grapheme_len(&runs[index].text);
        (field.put)(&mut runs[index].marks, value.clone());
        left -= len;
        index += 1;
    }
    normalize_runs(runs);
    Ok(())
}
