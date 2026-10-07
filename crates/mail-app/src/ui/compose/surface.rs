//! The message body: quire's `EditSurface` as `.c-body` itself, holding the paragraphs
//! `render.rs` draws, with the page's own caret and selection over them.
//!
//! The surface owns the focus and the IME and hands over input; it never edits text and never
//! draws a caret. Each input goes, in order:
//!
//! 1. through `body::key_taken`: an open menu's arrows,
//!    Enter and Escape, and the Ctrl chords (bold, italic, underline, undo, redo…);
//! 2. through [`adapt::asked`], to an editor event, a caret move or a clipboard gesture;
//! 3. an editor event into `wire::hear`: the IME rule, the
//!    page's own selection as the range, the `/` and `@` menus following.
//!
//! The caret and the selection are the page's (`Page::session.caret`, `Page::selection`), and
//! are drawn a frame after each change (`follow`) from the rects the surface's handle reports, less the
//! corner the marks layers are placed from (`.c-edit`'s, where the floats are placed too): the
//! selection in a layer before the surface, which Blitz paints under the positioned surface's
//! text as CSS 2.1 stacks them, and the caret and the IME's preedit in one after it. The caret is
//! drawn at its rect as given, `--caret-w` wide. The caret's rect in the window is also where the
//! IME's candidate window goes and where the `/` and `@` menus float (`Marks::at`).
//!
//! Spelling is quire's: the surface checks, marks and offers suggestions itself (`spell.rs` says
//! with which checker). The page hands it the setting, its caret, so the word being typed waits
//! until the caret leaves it, and takes a picked suggestion back as one editor edit.

use ds::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;
use ds::edit::handle::{EditHandle, use_edit_handle};
use ds::edit::input::{Composition, EditInput};
use ds::edit::pointer::{EditFocus, EditPointer};
use ds::host::position::{TextPosition, TextRange};
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::spell::marks::SpellReplace;

use super::adapt::{self, Asked, Reach, Step};
use super::body::{key_taken, now_ms};
use super::desk::Desk;
use super::float::{self, suggest_mention};
use super::page::Page;
use super::render;
use super::wire::{self, Heard};
use crate::ui::editor::{Caret, Doc, InputEvent, Pos, Range};
use crate::ui::host::Host;
use crate::ui::view::Shell;

/// Frames a measure waits for the document to be laid out and free.
const TRIES: usize = 24;

/// Gap between the caret and a float below it, and between a selection and the bubble above it.
const GAP: f32 = 6.0;

/// The caret and the selection as the surface last measured them.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Marks {
    /// The caret in the window's coordinates: where the IME's candidates and the menus go.
    pub at: Option<Rect>,
    /// The caret from the marks' corner.
    pub caret: Option<Rect>,
    /// The selection's boxes from the marks' corner, one per line and one per object.
    pub selection: Vec<Rect>,
    /// The height the bubble's distance from the bottom is measured against: from the marks'
    /// corner to the surface's bottom.
    pub height: f32,
}

impl Marks {
    /// A float's place under the caret, as `.c-float`'s inline style.
    pub(super) fn below_caret(&self) -> Option<String> {
        let caret = self.caret?;
        Some(format!(
            "left:{:.1}px;top:{:.1}px",
            caret.origin.x.0,
            caret.origin.y.0 + caret.size.height.0 + GAP
        ))
    }

    /// The bubble's place, above the selection's first line, as its inline style.
    pub(super) fn above_selection(&self) -> Option<String> {
        let first = self.selection.first()?;
        Some(format!(
            "left:{:.1}px;top:auto;bottom:{:.1}px",
            first.origin.x.0,
            self.height - first.origin.y.0 + GAP
        ))
    }
}

/// A box's inline style inside the marks layer.
fn boxed(rect: &Rect) -> String {
    format!(
        "left:{:.2}px;top:{:.2}px;width:{:.2}px;height:{:.2}px",
        rect.origin.x.0, rect.origin.y.0, rect.size.width.0, rect.size.height.0
    )
}

#[component]
pub(super) fn Surface(
    page: Signal<Page>,
    shell: Signal<Shell>,
    on_attach: EventHandler<()>,
    class: &'static str,
    marks: Signal<Marks>,
) -> Element {
    let handle = use_edit_handle();
    let mut focus = use_signal(|| EditFocus::Out);
    // A reply, or an addressed message, opens with the keyboard in its body
    // (`super::opening`). Once, after the surface has mounted: the page is peeked, not read.
    use_effect(move || {
        if super::opening(&page.peek()) == super::Opening::Body {
            handle.focus();
        }
    });
    let preedit = use_signal(|| None::<String>);
    // Set while the surface hands over a key the menus or a chord took, so the key goes no
    // further (Escape closes the menu, it does not also park the draft).
    let taken = use_hook(|| Rc::new(Cell::new(false)));
    // The selection's layer: its corner is the one every mark and float is placed from.
    let mut corner = use_signal(|| None::<Rc<MountedData>>);
    // Measure a frame after every change to the page.
    use_effect(move || {
        // Subscribes the effect to the page: the document, the caret and the selection.
        drop(page.read());
        spawn(follow(handle, corner, page, marks));
    });

    // Spelling: quire checks and marks; the word at the caret stays unmarked while it is typed.
    let spell = super::spell::spell_of(
        try_use_context::<Desk>()
            .map(|desk| (desk.spelling)())
            .unwrap_or_default(),
    );
    let at_caret = {
        let read = page.read();
        adapt::text_position(&read.session.doc, read.session.caret.pos)
    };
    let shown = marks.read().clone();
    let showing = preedit.read().clone();
    let caret = shown
        .caret
        .filter(|_| focus() == EditFocus::In && shown.selection.is_empty() && showing.is_none());
    let keys = Rc::clone(&taken);
    rsx! {
        div {
            onkeydown: move |event: KeyboardEvent| {
                if keys.replace(false) {
                    event.prevent_default();
                    event.stop_propagation();
                }
            },
            // Under the text: the selection, before the surface, neither with a z-index.
            div { class: "c-marks", aria_hidden: "true",
                onmounted: move |event| corner.set(Some(event.data())),
                for (index, rect) in shown.selection.iter().enumerate() {
                    div { key: "{index}", class: "c-sel", style: boxed(rect) }
                }
            }
            EditSurface {
                common: Common {
                    aria_label: Some("Message".to_owned()),
                    extra_class: ExtraClass::parse(class).ok(),
                    ..Common::default()
                },
                handle,
                ime_area: shown.at,
                on_input: move |input: EditInput| {
                    heard(page, shell, on_attach, handle, preedit, &taken, input);
                },
                on_pointer: move |pointer: EditPointer| pointed(page, &pointer),
                on_focus: move |now: EditFocus| focus.set(now),
                spell,
                caret: Some(at_caret),
                on_replace: move |replace: SpellReplace| replaced(page, replace),
                {render::body(page)}
            }
            // Over the text: the caret, and what the IME is composing, at the caret.
            div { class: "c-marks", aria_hidden: "true",
                if let Some(rect) = caret {
                    // At its rect as given: `--caret-w` wide, whole device pixels.
                    div { class: "c-caret", style: boxed(&rect) }
                }
                if let (Some(text), Some(rect)) = (showing, shown.caret) {
                    span {
                        class: "c-preedit",
                        style: "left:{rect.origin.x.0:.2}px;top:{rect.origin.y.0:.2}px;line-height:{rect.size.height.0:.2}px",
                        "{text}"
                    }
                }
            }
        }
    }
}

/// How far a [`follow`] has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    /// Nothing measured yet: the document is busy or not laid out.
    None,
    /// Measured once; read again a frame later in case that layout was not yet this change's.
    Once,
}

/// Draw the marks where the page's caret and selection are, a frame after a change and again a
/// frame after that.
///
/// Each read takes the caret and the selection as the page holds them when it reads, never as
/// they were when the change was made: a key typed while a read waits is drawn by it instead of
/// throwing it away. (Throwing it away, as a read of the change's own caret had to, left the
/// caret where typing began for as long as keys came closer than the wait, a held key's repeat
/// included.) The wait is one frame: the geometry is the last layout's, and the window lays the
/// change out on its next frame. The second read puts right a first one that ran before that
/// frame was drawn.
async fn follow(
    handle: EditHandle,
    corner: Signal<Option<Rc<MountedData>>>,
    page: Signal<Page>,
    mut marks: Signal<Marks>,
) {
    let mut read = Read::None;
    for _ in 0..TRIES {
        ds::base::time::clock::sleep(ds::base::time::FRAME_TICK).await;
        let Ok(now) = page.try_peek().map(|page| wanted(&page)) else {
            return;
        };
        let origin = corner.peek().as_deref().and_then(origin_of);
        let Some(measured) =
            origin.and_then(|at| read_marks(handle, at, &now.caret, now.range.as_ref()))
        else {
            continue;
        };
        if *marks.peek() != measured {
            marks.set(measured);
        }
        match read {
            Read::None => read = Read::Once,
            Read::Once => return,
        }
    }
}

/// Where the page wants its marks: the caret, and the selection when there is one.
struct Wanted {
    caret: TextPosition,
    range: Option<TextRange>,
}

fn wanted(page: &Page) -> Wanted {
    let doc = &page.session.doc;
    Wanted {
        caret: adapt::text_position(doc, page.session.caret.pos),
        range: page.selection.map(|range| TextRange {
            anchor: adapt::text_position(doc, range.start),
            focus: adapt::text_position(doc, range.end),
        }),
    }
}

/// Where `layer` is in the window: the corner of the box the marks and the floats are placed
/// in. `.c-body` hangs its gutter outside that box (its negative margin), so the surface's own
/// corner is not it.
fn origin_of(layer: &MountedData) -> Option<Point> {
    let host = use_document_host();
    match host.geometry().measure(layer) {
        Measured::At(rect) => Some(rect.origin),
        Measured::Busy | Measured::Unknown => None,
    }
}

/// One try at the marks: `None` while the document is busy or not laid out yet.
fn read_marks(
    handle: EditHandle,
    origin: Point,
    caret: &TextPosition,
    range: Option<&TextRange>,
) -> Option<Marks> {
    let bounds = handle.bounds().found()?;
    let at = handle.caret_rect(caret).found()?;
    let selection = match range {
        Some(range) => handle.selection_rects(range).found()?,
        None => Vec::new(),
    };
    let inside = |rect: Rect| Rect {
        origin: Point {
            x: Px(rect.origin.x.0 - origin.x.0),
            y: Px(rect.origin.y.0 - origin.y.0),
        },
        size: rect.size,
    };
    Some(Marks {
        at: Some(at),
        caret: Some(inside(at)),
        selection: selection.into_iter().map(inside).collect(),
        height: bounds.origin.y.0 + bounds.size.height.0 - origin.y.0,
    })
}

/// One input from the surface.
fn heard(
    page: Signal<Page>,
    shell: Signal<Shell>,
    on_attach: EventHandler<()>,
    handle: EditHandle,
    mut preedit: Signal<Option<String>>,
    taken: &Cell<bool>,
    input: EditInput,
) {
    if let EditInput::Key(key) = &input
        && key_taken(page, shell, on_attach, &key.key.to_string(), key.modifiers)
    {
        taken.set(true);
        return;
    }
    match &input {
        EditInput::Composition(Composition::Update { text, .. }) => {
            preedit.set(Some(text.clone()).filter(|text| !text.is_empty()));
        }
        EditInput::Composition(_) => preedit.set(None),
        _ => {}
    }
    match adapt::asked(&input) {
        Asked::Edit(event) => edit(page, event),
        Asked::Move(step, reach) => move_caret(page, handle, step, reach),
        Asked::SelectAll => {
            let (start, end) = {
                let read = page.peek();
                let doc = &read.session.doc;
                (
                    adapt::step(doc, Pos::new(0, 0), Step::DocStart),
                    adapt::step(doc, Pos::new(0, 0), Step::DocEnd),
                )
            };
            select(page, start, end);
        }
        Asked::Copy => {
            copy(page);
        }
        Asked::Cut => {
            if copy(page) {
                edit(
                    page,
                    InputEvent::new("deleteByCut", None, Vec::new(), false),
                );
            }
        }
        Asked::Nothing => {}
    }
}

/// An editor event, handed to the page: on the page's own selection,
/// through the IME rule, and the `@` menu asking the contact book what follows it.
fn edit(mut page: Signal<Page>, event: InputEvent) {
    let store = try_consume_context::<Arc<mail_store::SqliteStore>>();
    let mut write = page.write();
    let caret = write.session.caret.pos;
    let selection = write.selection.unwrap_or(Range {
        start: caret,
        end: caret,
    });
    let seq = write.wire.seq.wrapping_add(1);
    wire::hear(
        &mut write,
        Heard::Input {
            seq,
            event,
            selection: Some(selection),
        },
        now_ms(),
    );
    if let Some(store) = store {
        suggest_mention(&mut write, store.as_ref());
    }
}

/// A suggestion picked from the spelling menu: the misspelt word, as quire's byte range, replaced
/// by one `insertReplacementText` over the same graphemes, so it is one step for Ctrl Z.
fn replaced(page: Signal<Page>, replace: SpellReplace) {
    let range = {
        let read = page.peek();
        let doc = &read.session.doc;
        match (
            adapt::pos_of(doc, &replace.range.anchor),
            adapt::pos_of(doc, &replace.range.focus),
        ) {
            (Some(start), Some(end)) => Range { start, end }.ordered(),
            _ => return,
        }
    };
    edit(
        page,
        InputEvent::new(
            "insertReplacementText",
            Some(replace.text),
            vec![range],
            false,
        ),
    );
}

/// Where a selection that ends at the caret started: its other end, or the caret itself.
fn anchor_of(page: &Page) -> Pos {
    let caret = page.session.caret.pos;
    match page.selection {
        Some(range) if range.start == caret => range.end,
        Some(range) => range.start,
        None => caret,
    }
}

/// Select from `anchor` to `focus`, the caret at `focus`, with the direction kept. Nothing moves while the IME composes.
fn select(mut page: Signal<Page>, anchor: Pos, focus: Pos) {
    let range = Range {
        start: anchor,
        end: focus,
    };
    let selection = (!range.is_collapsed()).then(|| range.ordered());
    {
        let read = page.peek();
        if read.wire.composing.is_some()
            || (read.selection == selection && read.session.caret.pos == focus)
        {
            return;
        }
    }
    let mut write = page.write();
    write.selection = selection;
    if write.session.caret.pos != focus {
        write.session.caret = Caret::at(focus.node, focus.offset);
    }
    float::after_move(&mut write);
}

/// An arrow, Home or End.
fn move_caret(page: Signal<Page>, handle: EditHandle, step: Step, reach: Reach) {
    let (to, anchor) = {
        let read = page.peek();
        let doc = &read.session.doc;
        let caret = read.session.caret.pos;
        let to = match (step, read.selection, reach) {
            // A selection collapses to the end the arrow points at, as a browser's does.
            (Step::Left, Some(range), Reach::Collapse) => range.start,
            (Step::Right, Some(range), Reach::Collapse) => range.end,
            (Step::Up | Step::Down | Step::LineStart | Step::LineEnd, _, _) => {
                line_step(handle, doc, caret, step)
            }
            _ => adapt::step(doc, caret, step),
        };
        let anchor = match reach {
            Reach::Extend => anchor_of(&read),
            Reach::Collapse => to,
        };
        (to, anchor)
    };
    select(page, anchor, to);
}

/// A move over the laid-out lines, through the surface's geometry: the line above or below at
/// the caret's x, or the start or end of the caret's line. Where there is no line above (or
/// below), the document's start (or end); where the geometry cannot be read, the caret stays.
fn line_step(handle: EditHandle, doc: &Doc, from: Pos, step: Step) -> Pos {
    let Some(caret) = handle.caret_rect(&adapt::text_position(doc, from)).found() else {
        return from;
    };
    let x = caret.origin.x.0;
    let middle = caret.origin.y.0 + caret.size.height.0 / 2.0;
    let line = caret.size.height.0.max(1.0);
    let hit = |x: f32, y: f32| {
        handle
            .hit_test(Point { x: Px(x), y: Px(y) })
            .found()
            .and_then(|at| adapt::pos_of(doc, &at))
    };
    let before = |a: Pos, b: Pos| (a.node, a.offset) < (b.node, b.offset);
    match step {
        Step::Up | Step::Down => {
            let up = step == Step::Up;
            // A line and a half, two, three: past a paragraph's margin, or a heading's.
            for lines in [1.0, 1.5, 2.0, 3.0] {
                let y = if up {
                    middle - line * lines
                } else {
                    middle + line * lines
                };
                match hit(x, y) {
                    Some(to) if up && before(to, from) => return to,
                    Some(to) if !up && before(from, to) => return to,
                    _ => {}
                }
            }
            let edge = if up { Step::DocStart } else { Step::DocEnd };
            adapt::step(doc, from, edge)
        }
        Step::LineStart | Step::LineEnd => {
            let Some(bounds) = handle.bounds().found() else {
                return from;
            };
            let x = if step == Step::LineStart {
                bounds.origin.x.0 + 1.0
            } else {
                bounds.origin.x.0 + bounds.size.width.0 - 1.0
            };
            hit(x, middle).unwrap_or(from)
        }
        _ => adapt::step(doc, from, step),
    }
}

/// A press, drag or release over the surface: the caret, a selection from its anchor, a word
/// (double click) or a paragraph (triple). A point over nothing addressable keeps the caret.
fn pointed(page: Signal<Page>, pointer: &EditPointer) {
    let chosen = {
        let read = page.peek();
        adapt::pointer_selection(&read.session.doc, anchor_of(&read), pointer)
    };
    if let Some((anchor, focus)) = chosen {
        select(page, anchor, focus);
    }
}

/// Put the selection's text on the clipboard. `false` when nothing is selected.
fn copy(page: Signal<Page>) -> bool {
    let text = {
        let read = page.peek();
        let Some(range) = read.selection else {
            return false;
        };
        adapt::selected_text(&read.session.doc, range)
    };
    Host::copy(&text);
    true
}
