//! The words selected in a message's frame, for Copy and Select All.
//!
//! A message's body is a frame with a document of its own (`blocks::Sandbox`), and Blitz keeps a
//! selection per document: a drag in a message selects in that message's frame, and ⌘C there
//! copies it (Blitz's own, for the document the keyboard is in). What the window must add is the
//! rest of what a Mac reader does with those words: a right click on them offers Copy, and ⌘A
//! with the keyboard in the message selects the message, not every conversation in the list.
//!
//! The frame's document is reached through the frame's own node (`NodeHandle`). The window's
//! document is borrowed while an event is dispatched, and a task a handler spawns can first run
//! inside that dispatch, so each read or change here runs from a task, once the document is free
//! ([`free`]).
//!
//! A right press in a frame's words puts the frame's caret where it landed, as a left press does
//! (Blitz's `handle_pointerdown` asks no button), so the selection is gone by the time the
//! context menu is asked. The frame keeps what was selected when the last left press let go
//! ([`Held`]); a right click puts that back, and offers it to copy.

use blitz_dom::{BaseDocument, NodeId};
use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use ds::base::time::clock::sleep;
use std::time::Duration;

/// A selection in a frame's document: where it starts and ends (an inline root and a byte offset
/// into its text, each) and the words it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Held {
    from: (NodeId, usize),
    to: (NodeId, usize),
    pub(super) text: String,
}

/// What `frame`'s document has selected, when it is more than space.
pub(super) async fn selected(frame: &MountedData) -> Option<Held> {
    let handle = free(frame).await?;
    let doc = handle.try_doc()?;
    let sub = doc.subdoc(handle.node_id())?;
    let inner = sub.inner();
    held_in(&inner)
}

/// Select `held` again in `frame`'s document, if its words are still there.
pub(super) async fn reselect(frame: &MountedData, held: &Held) {
    change(frame, |doc| {
        let there = |id: NodeId| {
            doc.get_node(id)
                .is_some_and(|node| node.flags.is_inline_root())
        };
        if there(held.from.0) && there(held.to.0) {
            doc.set_text_selection(held.from.0, held.from.1, held.to.0, held.to.1);
        }
    })
    .await;
}

/// Select every word of `frame`'s document, from the start of its first run of text to the end
/// of its last, as ⌘A selects a page; what is selected then.
pub(super) async fn select_all(frame: &MountedData) -> Option<Held> {
    change(frame, |doc| {
        let roots = inline_roots(doc);
        if let (Some(&(first, _)), Some(&(last, len))) = (roots.first(), roots.last()) {
            doc.set_text_selection(first, 0, last, len);
        }
        held_in(doc)
    })
    .await
    .flatten()
}

/// What `doc` has selected, when it is more than space.
fn held_in(doc: &BaseDocument) -> Option<Held> {
    let text = doc
        .get_selected_text()
        .filter(|text| !text.trim().is_empty())?;
    let ranges = doc.get_text_selection_ranges();
    let (first, last) = (ranges.first()?, ranges.last()?);
    Some(Held {
        from: (first.0, first.1),
        to: (last.0, last.2),
        text,
    })
}

/// How long a task waits for the window's document to be free, a frame at a time, before it
/// gives up.
const TRIES: usize = 30;
const FRAME_TIME: Duration = Duration::from_millis(16);

/// `frame`'s node, once the window's document is free to borrow; `None` if it is not a node of
/// a Blitz document, or never comes free.
async fn free(frame: &MountedData) -> Option<&NodeHandle> {
    let handle = frame.downcast::<NodeHandle>()?;
    for _ in 0..TRIES {
        if handle.try_doc().is_some() {
            return Some(handle);
        }
        sleep(FRAME_TIME).await;
    }
    None
}

/// Change `frame`'s document with `change` once the window's document is free, and draw it
/// again; what `change` said, or `None` when there was no document to change.
async fn change<T>(frame: &MountedData, change: impl FnOnce(&mut BaseDocument) -> T) -> Option<T> {
    let handle = free(frame).await?;
    // Free a moment ago, and nothing has run since: no await lies between.
    let mut doc = handle.doc_mut();
    let sub = doc.subdoc_mut(handle.node_id())?;
    let mut inner = sub.inner_mut();
    let said = change(&mut inner);
    inner.shell_provider.request_redraw();
    Some(said)
}

/// The most nodes a frame is walked through for its text: a page could be very large.
const NODES: usize = 50_000;

/// Every inline root of `doc` that holds text, in the order it is drawn, with its text's length:
/// the boxes a selection's ends sit in. Walked as laid out, so the anonymous blocks Blitz wraps
/// bare text in are among them.
fn inline_roots(doc: &BaseDocument) -> Vec<(NodeId, usize)> {
    let mut found = Vec::new();
    let mut stack = vec![doc.root_element().id];
    let mut seen = 0;
    while let Some(id) = stack.pop() {
        seen += 1;
        if seen > NODES {
            break;
        }
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.flags.is_inline_root() {
            let len = node
                .element_data()
                .and_then(|data| data.inline_layout_data.as_ref())
                .map_or(0, |layout| layout.text.len());
            if len > 0 {
                found.push((id, len));
            }
            continue;
        }
        let children = node.layout_children.borrow();
        let children = children.as_deref().unwrap_or(&node.children[..]);
        stack.extend(children.iter().rev().copied());
    }
    found
}
