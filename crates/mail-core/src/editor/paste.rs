//! A paste: clipboard HTML or plain text, turned into edits at the selection.

use mail_mime::{RemoteImages, SanitizePolicy, from_html, sanitize};

use crate::editor::convert::{nodes_from_blocks, nodes_from_plain};
use crate::editor::doc::{Doc, Node, ParaKind, Pos, Range};
use crate::editor::error::OpError;
use crate::editor::input::{Edit, InputEvent, Record, at, without_selection};
use crate::editor::keys::Caret;
use crate::editor::op::Op;
use crate::editor::text::{grapheme_len, node_len};

/// Clipboard HTML goes through `sanitize`, then `from_html`, then nodes. Plain text is
/// paragraphs. One plain paragraph is typed into the line; anything more is inserted as nodes.
pub(crate) fn paste(doc: &Doc, selection: Range, event: &InputEvent) -> Result<Edit, OpError> {
    let nodes = match &event.html {
        Some(html) => {
            let safe = sanitize(html, SanitizePolicy::CURRENT);
            nodes_from_blocks(&from_html(&safe, &[], RemoteImages::Blocked).blocks)
        }
        None => nodes_from_plain(event.data.as_deref().unwrap_or("")),
    };
    let (mut ops, after) = without_selection(doc, selection)?;
    let start = selection.start;
    let caret = match nodes.as_slice() {
        [] => at(start),
        [
            Node::Para {
                kind: ParaKind::Paragraph,
                runs,
            },
        ] if matches!(after.nodes.get(start.node), Some(Node::Para { .. })) => {
            let mut offset = start.offset;
            for run in runs {
                ops.push(Op::Insert {
                    at: Pos::new(start.node, offset),
                    text: run.text.clone(),
                    marks: run.marks.clone(),
                });
                offset += grapheme_len(&run.text);
            }
            Caret::at(start.node, offset)
        }
        _ => {
            let last = nodes.len() - 1;
            let last_len = node_len(&nodes[last]);
            let first = first_inserted(&after, start);
            ops.push(Op::InsertNodes { at: start, nodes });
            Caret::at(first + last, last_len)
        }
    };
    Ok(Edit {
        record: if ops.is_empty() {
            Record::Skip
        } else {
            Record::Structural
        },
        ops,
        caret,
    })
}

/// Where the first of the nodes [`Op::InsertNodes`] puts at `start` lands.
fn first_inserted(doc: &Doc, start: Pos) -> usize {
    match doc.nodes.get(start.node) {
        None => start.node,
        Some(node @ Node::Para { .. }) if node_len(node) == 0 => start.node,
        Some(_) if start.offset == 0 => start.node,
        Some(_) => start.node + 1,
    }
}
