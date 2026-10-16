//! An earlier message's frame as tall as what it holds, as Mail draws every message of a
//! conversation in full: the page scrolls, never a box inside it.
//!
//! The frame's document is read through the frame's own node on Blitz (`NodeHandle`): its root
//! element's box or its content's extent, whichever is taller, once laid out. A frame starts
//! short (`.html[data-fit]`), so a page that fills its viewport (`html { height: 100% }`) does not
//! measure as tall as a guess; it is read again while its pictures may still be arriving, and grows
//! with them.

use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use ds::base::time::clock::sleep;
use std::rc::Rc;
use std::time::Duration;

/// How often a frame is read again, and for how long after it mounts: long enough for the
/// pictures a person allowed to arrive.
const EVERY: Duration = Duration::from_millis(120);
const READS: usize = 80;

/// The frame's content height in CSS pixels, once its document is laid out.
fn content_height(frame: &MountedData) -> Option<f32> {
    let handle = frame.downcast::<NodeHandle>()?;
    let doc = handle.try_doc()?;
    let sub = doc.subdoc(handle.node_id())?;
    let inner = sub.inner();
    let root = inner.root_element();
    // The content's extent leaves out the body's own margins; the root's box holds them. Short of
    // either, the frame keeps a few pixels of range of its own, and a wheel burst that starts over
    // it stays with it instead of moving the reader.
    let layout = root.final_layout();
    let height = layout
        .scrollable_overflow_rect
        .bottom
        .max(layout.size.height);
    (height > 0.0).then_some(height.ceil())
}

/// Keep `height` at `frame`'s content height while it may still change.
pub(super) fn follow(frame: Rc<MountedData>, mut height: Signal<Option<f32>>) {
    spawn(async move {
        for _ in 0..READS {
            if let Some(read) = content_height(&frame)
                && height.peek().is_none_or(|was| (was - read).abs() >= 1.0)
            {
                height.set(Some(read));
            }
            sleep(EVERY).await;
        }
    });
}
