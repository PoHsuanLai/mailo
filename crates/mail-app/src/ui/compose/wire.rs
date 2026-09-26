//! Where an editor event meets the page: [`hear`] turns what the surface handed over
//! (`surface.rs`, through `adapt.rs`) into `editor::` calls on the page's session, with the IME
//! rule and the choice of which range an event acts on.

use super::float;
use super::page::Page;
use crate::editor::{InputEvent, Range};

/// An editor event, as the surface hands it to the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Heard {
    /// An input or a composition event, numbered.
    Input {
        seq: u64,
        event: InputEvent,
        /// The selection when it happened, or `None` to act at the page's own caret.
        selection: Option<Range>,
    },
}

/// Apply one message to the page. `at_ms` groups typing into undo steps.
pub(in crate::ui) fn hear(page: &mut Page, heard: Heard, at_ms: u64) {
    match heard {
        Heard::Input {
            seq,
            event,
            selection,
        } => {
            page.wire.seq = seq;
            input(page, event, selection, at_ms);
        }
    }
}

/// The IME rule, and the choice of which range an event acts on.
fn input(page: &mut Page, mut event: InputEvent, selection: Option<Range>, at_ms: u64) {
    let caret = page.session.caret.pos;
    let here = Range {
        start: caret,
        end: caret,
    };
    match event.input_type.as_str() {
        "compositionstart" => {
            page.wire.composing = Some(selection.unwrap_or(here));
            return;
        }
        "compositionend" => {
            let start = page.wire.composing.take().unwrap_or(here);
            event.ranges = vec![start];
            apply(page, &event, at_ms);
            // The IME wrote into this paragraph's DOM. A new key builds it again from the doc.
            let generation = page.fresh.entry(start.start.node).or_insert(0);
            *generation += 1;
            return;
        }
        _ if page.wire.composing.is_some() => return,
        _ => {}
    }
    if let Some(selection) = selection {
        // A collapsed Backspace or Delete is Rust's to resolve (an armed object, a heading that
        // gives up its kind first), not the browser's guess at one cluster.
        let resolves_itself = matches!(
            event.input_type.as_str(),
            "deleteContentBackward" | "deleteContentForward" | "deleteWordBackward"
        ) && selection.is_collapsed();
        if event.ranges.is_empty() || resolves_itself {
            event.ranges = vec![selection];
        }
    }
    apply(page, &event, at_ms);
}

fn apply(page: &mut Page, event: &InputEvent, at_ms: u64) {
    let before = page.session.doc.clone();
    let keeps_selection = event.input_type.starts_with("format");
    if let Err(why) = page.session.handle(event, at_ms) {
        page.notice = Some(format!("That edit did not apply: {why}"));
        return;
    }
    if !keeps_selection {
        page.selection = None;
    }
    if page.session.doc != before {
        page.touch();
    }
    float::after_input(page, event);
}
