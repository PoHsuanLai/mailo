//! An earlier message's frame as tall as what it holds, as Mail draws every message of a
//! conversation in full: the page scrolls, never a box inside it.
//!
//! The frame's document is read through the frame's own node on Blitz (`NodeHandle`): its root
//! element's box or its content's extent, whichever is taller, once laid out. A frame starts
//! short (`.html[data-fit]`), so a page that fills its viewport (`html { height: 100% }`) does not
//! measure as tall as a guess; it is read again while its pictures may still be arriving, and grows
//! with them.
//!
//! Every message's page, the last one's too, is also kept no wider than its frame: one a sender
//! laid out wider than the pane is drawn smaller to fit it ([`shrink`]), as Mail does, rather
//! than cut at its right edge.

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

/// The least a message is shrunk to: past this its words are too small to read, and the
/// frame's right edge cuts what is left over instead.
const LEAST: f32 = 0.4;

/// The zoom that fits `frame`'s document to the frame's width: the frame's width over the
/// document's own (a newsletter laid out for 800 px, its tables and pictures fixed at that width,
/// in a narrower pane), and 1 when it fits, so a pane widened while the page is still read lets
/// it grow back. The document's width is read in its own pixels, whatever zoom it is drawn at
/// now, so a read taken before a new zoom has landed asks for the same one. `None` when it is not
/// laid out.
fn fitting(frame: &MountedData) -> Option<f32> {
    let handle = frame.downcast::<NodeHandle>()?;
    let doc = handle.try_doc()?;
    let width = doc.get_node(handle.node_id())?.final_layout().size.width;
    let sub = doc.subdoc(handle.node_id())?;
    let inner = sub.inner();
    let drawn = laid_width(&inner)?;
    let own = inner
        .root_element()
        .primary_styles()
        .map_or(drawn, |style| style.effective_zoom.unzoom(drawn));
    if width <= 0.0 {
        return None;
    }
    Some(if own > width + 1.0 {
        (width / own).max(LEAST)
    } else {
        1.0
    })
}

/// How wide `doc` is laid out, in its pixels as drawn: its widest box under `<body>`, and the
/// body's own insets either side. Not where boxes end: Blitz starts a centred box that overflows
/// at an offset that does not shrink with the zoom, where its width does. Read to a bound, as a
/// page could be very large.
fn laid_width(doc: &blitz_dom::BaseDocument) -> Option<f32> {
    const BOXES: usize = 20_000;
    let root = doc.root_element();
    let body = root
        .children
        .iter()
        .filter_map(|id| doc.get_node(*id))
        .find(|node| {
            node.element_data()
                .is_some_and(|data| &*data.name.local == "body")
        })?;
    let outer = body.final_layout();
    let insets = outer.padding.left
        + outer.padding.right
        + outer.border.left
        + outer.border.right
        + outer.margin.left
        + outer.margin.right;
    let mut widest = 0.0_f32;
    let mut stack: Vec<_> = body.children.to_vec();
    let mut seen = 0;
    while let Some(id) = stack.pop() {
        seen += 1;
        if seen > BOXES {
            break;
        }
        // Text and comments have no box of their own.
        let Some(node) = doc
            .get_node(id)
            .filter(|node| node.is_element() || node.is_anonymous())
        else {
            continue;
        };
        widest = widest.max(node.final_layout().size.width);
        stack.extend(node.children.iter().copied());
    }
    Some(widest + insets)
}

/// Keep `zoom` at what fits `frame`'s document to its width, as Mail shrinks a message laid out
/// wider than its pane rather than cut its right edge, while its pictures may still widen it.
pub(super) fn shrink(frame: Rc<MountedData>, mut zoom: Signal<f32>) {
    spawn(async move {
        for _ in 0..READS {
            if let Some(next) = fitting(&frame)
                && (next - *zoom.peek()).abs() >= 0.01
            {
                zoom.set(next);
            }
            sleep(EVERY).await;
        }
    });
}

/// `html`, the frame's document, drawn at `zoom`: unchanged at 1.
pub(super) fn zoomed(html: &str, zoom: f32) -> String {
    if zoom >= 1.0 {
        return html.to_owned();
    }
    html.replacen(
        "</head>",
        &format!("<style>html {{ zoom: {zoom:.3}; }}</style></head>"),
        1,
    )
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

#[cfg(test)]
mod tests {
    use super::zoomed;

    const PAGE: &str =
        "<!DOCTYPE html><html><head><style>p {}</style></head><body><p>Hi</p></body></html>";

    #[test]
    fn a_page_that_fits_is_its_own_size_and_a_wider_one_is_zoomed_after_its_sheets() {
        assert_eq!(zoomed(PAGE, 1.0), PAGE);
        assert_eq!(
            zoomed(PAGE, 0.6814),
            "<!DOCTYPE html><html><head><style>p {}</style><style>html { zoom: 0.681; }</style>\
             </head><body><p>Hi</p></body></html>"
        );
    }
}
