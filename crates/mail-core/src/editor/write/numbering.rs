//! How numbered items are counted: the one rule behind the number the plain text writes
//! (`format=flowed`) and the number the editor draws.

use super::{ListTag, same_list};
use crate::editor::doc::{Node, ParaKind};

/// The number each numbered item carries, by node: `None` for a node that is not one.
///
/// Counted per depth through a run of items, so a list that goes on after a nested one goes on
/// counting. A node that is not a list item ends the run. An item is counted at the depth it is
/// written at ([`super::written_depths`]): its own, but at most one below the item before it, as
/// a list nests inside an item and cannot skip one. An item of another list at its depth
/// starts that list over.
pub fn item_numbers(nodes: &[Node]) -> Vec<Option<usize>> {
    let mut out = vec![None; nodes.len()];
    let mut counts: Vec<(ListTag, usize)> = Vec::new();
    let mut before: Option<usize> = None;
    for (index, node) in nodes.iter().enumerate() {
        let item = match node {
            Node::Para { kind, layout, .. } => {
                ListTag::of(*kind).map(|tag| (tag, *kind, usize::from(layout.depth)))
            }
            Node::Object(_) => None,
        };
        let Some((tag, kind, depth)) = item else {
            counts.clear();
            before = None;
            continue;
        };
        let depth = depth.min(before.map_or(0, |above| above + 1));
        before = Some(depth);
        counts.truncate(depth + 1);
        let number = match counts.get_mut(depth) {
            Some((last, count)) if same_list(*last, tag) => {
                *count += 1;
                *count
            }
            Some(slot) => {
                *slot = (tag, 1);
                1
            }
            None => {
                counts.push((tag, 1));
                1
            }
        };
        if kind == ParaKind::Numbered {
            out[index] = Some(number);
        }
    }
    out
}
