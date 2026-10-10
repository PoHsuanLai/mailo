//! Setting an inline mark or a link over a range, one paragraph at a time.

use mail_mime::SafeUrl;

use super::{Op, check_range, covers, one_or_seq, paragraph_mut};
use crate::editor::doc::{Doc, Mark, Node, Pos, Presence, Range, Run};
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
            differing_links(runs, from, to, &url)
        };
        let runs = paragraph_mut(doc, Pos::new(index, 0))?;
        paint_link(runs, from, to, url.clone())?;
        for (start, end, previous) in segments {
            inverses.push(Op::SetLink {
                range: Range {
                    start: Pos::new(index, start),
                    end: Pos::new(index, end),
                },
                url: previous,
            });
        }
    }
    Ok(one_or_seq(inverses, Op::SetLink { range, url }))
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

fn differing_links(
    runs: &[Run],
    from: usize,
    to: usize,
    url: &Option<SafeUrl>,
) -> Vec<(usize, usize, Option<SafeUrl>)> {
    let mut out: Vec<(usize, usize, Option<SafeUrl>)> = Vec::new();
    let mut seen = 0usize;
    for run in runs {
        let end = seen + grapheme_len(&run.text);
        let start = from.max(seen);
        let stop = to.min(end);
        if start < stop && run.marks.link != *url {
            let extend = out.last().is_some_and(|(_, prev_end, previous)| {
                *prev_end == start && previous == &run.marks.link
            });
            if extend {
                if let Some((_, prev_end, _)) = out.last_mut() {
                    *prev_end = stop;
                }
            } else {
                out.push((start, stop, run.marks.link.clone()));
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

fn paint_link(
    runs: &mut Vec<Run>,
    from: usize,
    to: usize,
    url: Option<SafeUrl>,
) -> Result<(), OpError> {
    if from == to {
        return Ok(());
    }
    split_at(runs, to)?;
    let mut index = split_at(runs, from)?;
    let mut left = to - from;
    while left > 0 {
        let len = grapheme_len(&runs[index].text);
        runs[index].marks.link = url.clone();
        left -= len;
        index += 1;
    }
    normalize_runs(runs);
    Ok(())
}
