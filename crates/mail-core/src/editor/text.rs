//! Text inside a paragraph: grapheme counting, and cutting runs at grapheme offsets.
//!
//! Every offset here counts extended grapheme clusters (Unicode Standard Annex #29), run by
//! run. A cluster never straddles two runs, because runs are only ever cut at a cluster
//! boundary of their own text.

use unicode_segmentation::UnicodeSegmentation;

use crate::editor::doc::{Mark, Marks, Node, Run};
use crate::editor::error::OpError;

/// How many grapheme clusters `text` contains.
pub fn grapheme_len(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Byte index of grapheme cluster `grapheme`, or `text.len()` when `grapheme` is the length.
pub fn byte_at(text: &str, grapheme: usize) -> Option<usize> {
    let mut seen = 0usize;
    for (byte, _) in text.grapheme_indices(true) {
        if seen == grapheme {
            return Some(byte);
        }
        seen += 1;
    }
    if seen == grapheme {
        Some(text.len())
    } else {
        None
    }
}

/// Grapheme length of a paragraph's runs.
pub fn para_len(runs: &[Run]) -> usize {
    runs.iter().map(|run| grapheme_len(&run.text)).sum()
}

/// Grapheme length of a node. Objects are one cluster long.
pub fn node_len(node: &Node) -> usize {
    match node {
        Node::Para { runs, .. } => para_len(runs),
        Node::Object(_) => 1,
    }
}

/// The paragraph's text, runs joined.
pub fn runs_text(runs: &[Run]) -> String {
    let mut out = String::new();
    for run in runs {
        out.push_str(&run.text);
    }
    out
}

/// Drop empty runs and join neighbours that carry the same marks.
pub fn normalize_runs(runs: &mut Vec<Run>) {
    runs.retain(|run| !run.text.is_empty());
    let mut index = 0;
    while index + 1 < runs.len() {
        if runs[index].marks == runs[index + 1].marks {
            let next = runs.remove(index + 1);
            runs[index].text.push_str(&next.text);
        } else {
            index += 1;
        }
    }
}

/// Split so a run boundary falls at `grapheme`. Returns the run index that starts there.
pub(crate) fn split_at(runs: &mut Vec<Run>, grapheme: usize) -> Result<usize, OpError> {
    let mut seen = 0usize;
    let mut index = 0;
    while index < runs.len() {
        if seen == grapheme {
            return Ok(index);
        }
        let len = grapheme_len(&runs[index].text);
        if seen + len > grapheme {
            let byte = byte_at(&runs[index].text, grapheme - seen).ok_or(OpError::OutOfRange)?;
            let right = runs[index].text.split_off(byte);
            let marks = runs[index].marks.clone();
            runs.insert(index + 1, Run { text: right, marks });
            return Ok(index + 1);
        }
        seen += len;
        index += 1;
    }
    if seen == grapheme {
        Ok(runs.len())
    } else {
        Err(OpError::OutOfRange)
    }
}

/// Insert `text` at a grapheme offset, then normalize.
pub(crate) fn insert_text(
    runs: &mut Vec<Run>,
    at: usize,
    text: &str,
    marks: Marks,
) -> Result<(), OpError> {
    if text.is_empty() {
        return Ok(());
    }
    if at > para_len(runs) {
        return Err(OpError::OutOfRange);
    }
    let index = split_at(runs, at)?;
    if index > 0 && runs[index - 1].marks == marks {
        runs[index - 1].text.push_str(text);
    } else {
        runs.insert(index, Run::new(text, marks));
    }
    normalize_runs(runs);
    Ok(())
}

/// Remove graphemes `[from, to)` and return what was removed, normalized.
pub(crate) fn delete_text(
    runs: &mut Vec<Run>,
    from: usize,
    to: usize,
) -> Result<Vec<Run>, OpError> {
    if from > to || to > para_len(runs) {
        return Err(OpError::OutOfRange);
    }
    if from == to {
        return Ok(Vec::new());
    }
    split_at(runs, to)?;
    let start = split_at(runs, from)?;
    let mut need = to - from;
    let mut removed = Vec::new();
    while need > 0 {
        if start >= runs.len() {
            return Err(OpError::OutOfRange);
        }
        let len = grapheme_len(&runs[start].text);
        if len > need {
            return Err(OpError::OutOfRange);
        }
        let run = runs.remove(start);
        need -= len;
        removed.push(run);
    }
    normalize_runs(runs);
    normalize_runs(&mut removed);
    Ok(removed)
}

/// Marks a typed character inherits: the cluster before the caret, else the one after.
pub fn marks_at(runs: &[Run], offset: usize) -> Marks {
    if runs.is_empty() {
        return Marks::new();
    }
    let mut seen = 0usize;
    let mut previous = None;
    for run in runs {
        let len = grapheme_len(&run.text);
        if offset < seen + len {
            if offset > seen {
                return run.marks.clone();
            }
            return previous.unwrap_or(&run.marks).clone();
        }
        seen += len;
        previous = Some(&run.marks);
    }
    previous.cloned().unwrap_or_else(Marks::new)
}

/// Whether every grapheme in `[from, to)` carries `mark`. An empty range does.
pub fn range_has_mark(runs: &[Run], from: usize, to: usize, mark: Mark) -> bool {
    if from >= to {
        return true;
    }
    let mut seen = 0usize;
    let mut covered = from;
    for run in runs {
        let len = grapheme_len(&run.text);
        let run_end = seen + len;
        let overlap_start = from.max(seen);
        let overlap_end = to.min(run_end);
        if overlap_start < overlap_end {
            if !run.marks.has(mark) {
                return false;
            }
            covered = overlap_end;
        }
        seen = run_end;
        if seen >= to {
            break;
        }
    }
    covered >= to
}
