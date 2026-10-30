//! ⌘F: find in the open conversation, in the words of its messages as they are drawn.
//!
//! Each message's body is a frame with a document of its own (`blocks::Sandbox`), reached through
//! the frame's node on Blitz (`NodeHandle`), as `fit` reads it. A find walks each shown frame's
//! text nodes in reading order, as runs of one block at a time (`mail_core::search::find_in`), and
//! puts a `<mark>` around each piece of each match ([`FOUND_MARK`]); the frame's own sheet draws
//! the marks, the current match stronger (`mail_core::message::frame`). The sender's markup is
//! never rewritten: the frame is still drawn from the sanitized document, and in its live tree a
//! matched text node is only stood aside while marks hold its words; closing the bar, or typing
//! something else, puts every one back.
//!
//! The current match is brought to the middle of what scrolls it: the frame's own document for
//! the last message, which scrolls inside its frame, then the reader around the frame. Where
//! the match sits is read from the line Blitz laid it out on.
//!
//! The window's document is borrowed while an event is dispatched, so every read or change here
//! runs from a task, once the document is free ([`free`]), and is one step with no await inside
//! it. A find started while an older one still waits supersedes it (`Inner::pass`).

use crate::ui::copy;
use crate::ui::host::Host;
use crate::ui::view::{Finding, Shell};
use blitz_dom::{Attribute, BaseDocument, LocalName, Namespace, NodeId, QualName, ScrollBehavior};
use chordkit::StandardAction;
use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use ds::base::time::{FRAME_SLACK, clock::sleep};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::focus::request::use_focus_request;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_client::search::{Found, MOST_FOUND, Step, find_in};
use mail_domain::MessageId;
use mail_reader::{FOUND_CURRENT, FOUND_MARK};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The bar's field, for focusing it again when ⌘F is pressed with the bar open.
const FIELD: &str = ".find-bar input";

/// The frames a reader draws, which a find looks through, and what it has marked in each.
#[derive(Clone)]
pub(super) struct Frames {
    inner: Rc<RefCell<Inner>>,
    /// Moved when a frame is drawn, drawn again or goes, so what is marked is marked again.
    changed: Signal<u64>,
    /// Moved when a find has marked every frame, so the current match is shown in the new marks.
    marked: Signal<u64>,
}

#[derive(Default)]
struct Inner {
    /// The shown messages, oldest first: the order matches are counted in.
    order: Vec<MessageId>,
    drawn: HashMap<MessageId, Rc<MountedData>>,
    marks: HashMap<MessageId, Marks>,
    /// Moved by every find, so one still waiting for a document gives way to a newer one.
    pass: u64,
}

/// What a find put in one frame's document.
#[derive(Debug, Default)]
struct Marks {
    /// The document they are in, by the stamp the find put on it ([`stamp`]): a frame drawn again
    /// holds a new document, which carries no stamp, and in which none of these nodes are.
    doc: u64,
    /// Each text node stood aside, and what stands in its place: its words cut at the marks.
    wraps: Vec<(NodeId, Vec<NodeId>)>,
    /// Each match's marks, in order.
    found: Vec<Vec<NodeId>>,
    /// The match drawn as the current one, counted in this frame.
    current: Option<usize>,
}

impl Frames {
    /// The shown messages, oldest first.
    pub(super) fn order(&self, order: Vec<MessageId>) {
        self.inner.borrow_mut().order = order;
    }

    /// `message`'s frame has mounted.
    pub(super) fn drawn(&self, message: MessageId, frame: Rc<MountedData>) {
        self.inner.borrow_mut().drawn.insert(message, frame);
        self.changed();
    }

    /// `message`'s frame is gone; nothing of it is marked any more.
    pub(super) fn gone(&self, message: MessageId) {
        let mut inner = self.inner.borrow_mut();
        inner.drawn.remove(&message);
        inner.marks.remove(&message);
    }

    /// A frame's document may have been drawn again: what was marked is marked again.
    pub(super) fn changed(&self) {
        let mut changed = self.changed;
        *changed.write() += 1;
    }

    /// The shown frames, in order.
    fn shown(&self) -> Vec<(MessageId, Rc<MountedData>)> {
        let inner = self.inner.borrow();
        inner
            .order
            .iter()
            .filter_map(|id| inner.drawn.get(id).map(|frame| (*id, Rc::clone(frame))))
            .collect()
    }

    /// Take every mark out and put `query`'s in, frame by frame; how many matches there are, or
    /// `None` when a newer find took over.
    async fn refind(&self, query: &str) -> Option<usize> {
        let pass = {
            let mut inner = self.inner.borrow_mut();
            inner.pass += 1;
            if query.trim().is_empty() && inner.marks.is_empty() {
                return Some(0);
            }
            inner.pass
        };
        // A frame drawn again is read once its new document is in.
        sleep(FRAME_SLACK).await;
        let mut total = 0;
        for (id, frame) in self.shown() {
            let handle = free(&frame).await;
            if self.inner.borrow().pass != pass {
                return None;
            }
            let Some(handle) = handle else {
                continue;
            };
            let old = self.inner.borrow_mut().marks.remove(&id);
            let marks = in_frame(handle, |doc| {
                if let Some(old) = &old {
                    unmark(doc, old);
                }
                mark(doc, query, MOST_FOUND - total.min(MOST_FOUND))
            });
            if let Some(marks) = marks.filter(|marks| !marks.found.is_empty()) {
                total += marks.found.len();
                self.inner.borrow_mut().marks.insert(id, marks);
            }
        }
        let mut marked = self.marked;
        *marked.write() += 1;
        Some(total)
    }

    /// Draw match `at`, counted across the frames, as the current one, and bring it into view.
    async fn point(&self, at: Option<usize>) {
        let mut before = 0;
        let mut target = None;
        for (id, frame) in self.shown() {
            let Some(handle) = free(&frame).await else {
                continue;
            };
            let mut inner = self.inner.borrow_mut();
            let Some(marks) = inner.marks.get_mut(&id) else {
                continue;
            };
            let count = marks.found.len();
            let here = at
                .and_then(|at| at.checked_sub(before))
                .filter(|local| *local < count);
            in_frame(handle, |doc| point_at(doc, marks, here));
            if let Some(local) = here {
                target = marks.found[local]
                    .first()
                    .map(|mark| (Rc::clone(&frame), *mark));
            }
            before += count;
        }
        let Some((frame, mark)) = target else {
            return;
        };
        // The marks are laid out by the next frame.
        for _ in 0..TRIES {
            sleep(FRAME_SLACK).await;
            let Some(handle) = free(&frame).await else {
                return;
            };
            if bring_into_view(handle, mark) {
                return;
            }
        }
    }
}

/// How many frames a task waits for a document to be free, or a mark to be laid out.
const TRIES: usize = 30;

/// `frame`'s node, once the window's document is free to borrow; `None` if it is not a node of a
/// Blitz document, or never comes free.
async fn free(frame: &MountedData) -> Option<&NodeHandle> {
    let handle = frame.downcast::<NodeHandle>()?;
    for _ in 0..TRIES {
        if handle.try_doc().is_some() {
            return Some(handle);
        }
        sleep(FRAME_SLACK).await;
    }
    None
}

/// Run `change` on the document in `handle`'s frame. Free a moment ago, and nothing has run
/// since: the caller has not awaited between.
fn in_frame<T>(handle: &NodeHandle, change: impl FnOnce(&mut BaseDocument) -> T) -> Option<T> {
    let mut doc = handle.doc_mut();
    let sub = doc.subdoc_mut(handle.node_id())?;
    let mut inner = sub.inner_mut();
    let said = change(&mut inner);
    inner.shell_provider.request_redraw();
    Some(said)
}

/// The attribute a find stamps its document's root element with, to know that document again.
const STAMP: &str = "data-found-doc";

/// The root element of `doc`, when it has one yet.
fn root_of(doc: &BaseDocument) -> Option<NodeId> {
    let root = doc.get_node(doc.root_node().id)?;
    root.children
        .iter()
        .copied()
        .find(|id| doc.get_node(*id).is_some_and(|node| node.is_element()))
}

/// Which document `doc` is: the stamp a find put on it, or 0 for a document none has marked.
/// A document drawn again is a new one with no stamp, even where the old one's memory was
/// reused, which an address could not tell apart.
fn identity(doc: &BaseDocument) -> u64 {
    root_of(doc)
        .and_then(|id| doc.get_node(id))
        .and_then(|node| node.attr(LocalName::from(STAMP)))
        .and_then(|stamp| stamp.parse().ok())
        .unwrap_or(0)
}

/// A stamp no earlier one was: 1 and up, never 0.
fn next_stamp() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    LAST.fetch_add(1, Ordering::Relaxed) + 1
}

/// An HTML element's or an attribute's name.
fn element(name: &str) -> QualName {
    QualName::new(
        None,
        Namespace::from("http://www.w3.org/1999/xhtml"),
        LocalName::from(name),
    )
}

fn attribute(name: &str) -> QualName {
    QualName::new(None, Namespace::from(""), LocalName::from(name))
}

/// The most nodes a frame is walked through for its text: a page could be very large.
const NODES: usize = 50_000;

/// `doc`'s text, as runs of the text nodes of one block, in reading order: what is laid out,
/// so a hidden preheader or a folded quote's text is left out, as it is not drawn.
fn runs(doc: &BaseDocument) -> Vec<Vec<(NodeId, String)>> {
    let mut groups: Vec<Vec<(NodeId, String)>> = Vec::new();
    let mut last_root = None;
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
        if node.is_element() && !node.has_boxes() {
            continue;
        }
        if let Some(text) = node.text_data() {
            let Some(root) = inline_root(doc, id) else {
                continue;
            };
            if last_root != Some(root) {
                groups.push(Vec::new());
                last_root = Some(root);
            }
            if let Some(group) = groups.last_mut() {
                group.push((id, text.content.clone()));
            }
            continue;
        }
        stack.extend(node.children.iter().rev().copied());
    }
    groups
}

/// The box `id`'s text is laid out in: its nearest inline root, through the boxes Blitz lays
/// out (an anonymous block around bare text among blocks) or, short of those, the elements.
fn inline_root(doc: &BaseDocument, id: NodeId) -> Option<NodeId> {
    let mut at = doc.get_node(id)?;
    for _ in 0..NODES {
        if at.flags.is_inline_root() {
            return Some(at.id);
        }
        at = doc.get_node(at.layout_parent.get().or(at.parent)?)?;
    }
    None
}

/// A text node a find cuts: its words, and each piece a match covers with the match's number.
type Cut = (NodeId, String, Vec<(Range<usize>, usize)>);

/// Mark `query`'s matches in `doc`, at most `most` of them.
fn mark(doc: &mut BaseDocument, query: &str, most: usize) -> Marks {
    let mut marks = Marks::default();
    if query.trim().is_empty() || most == 0 {
        return marks;
    }
    let Some(root) = root_of(doc) else {
        return marks;
    };
    let stamp = next_stamp();
    marks.doc = stamp;
    // Each text node a match is in, in reading order, with the pieces of it each match covers.
    let mut cuts: Vec<Cut> = Vec::new();
    let mut count = 0;
    for group in runs(doc) {
        let texts: Vec<&str> = group.iter().map(|(_, text)| text.as_str()).collect();
        let mut at: HashMap<usize, usize> = HashMap::new();
        for Found(pieces) in find_in(&texts, query) {
            if count == most {
                break;
            }
            for (run, bytes) in pieces {
                let slot = *at.entry(run).or_insert_with(|| {
                    cuts.push((group[run].0, group[run].1.clone(), Vec::new()));
                    cuts.len() - 1
                });
                cuts[slot].2.push((bytes, count));
            }
            count += 1;
        }
    }
    marks.found = vec![Vec::new(); count];
    let mut changes = doc.mutate();
    changes.set_attribute(root, attribute(STAMP), &stamp.to_string());
    for (node, text, pieces) in cuts {
        let mut parts = Vec::new();
        let mut at = 0;
        for (bytes, which) in pieces {
            if bytes.start > at {
                parts.push(changes.create_text_node(&text[at..bytes.start]));
            }
            let held = Attribute {
                name: attribute(FOUND_MARK),
                value: String::new(),
            };
            let mark = changes.create_element(element("mark"), vec![held]);
            let words = changes.create_text_node(&text[bytes.clone()]);
            changes.append_children(mark, &[words]);
            parts.push(mark);
            marks.found[which].push(mark);
            at = bytes.end;
        }
        if at < text.len() {
            parts.push(changes.create_text_node(&text[at..]));
        }
        changes.insert_nodes_before(node, &parts);
        changes.remove_node(node);
        marks.wraps.push((node, parts));
    }
    marks
}

/// Put back every text node `marks` stood aside, and take its marks out. Nothing in a document
/// drawn again, or where the nodes are no longer as they were left.
fn unmark(doc: &mut BaseDocument, marks: &Marks) {
    if marks.doc == 0 || marks.doc != identity(doc) {
        return;
    }
    let mut changes = doc.mutate();
    for (original, parts) in &marks.wraps {
        let Some(&first) = parts.first() else {
            continue;
        };
        let placed = changes
            .doc
            .get_node(first)
            .is_some_and(|node| node.parent.is_some());
        let aside = changes
            .doc
            .get_node(*original)
            .is_some_and(|node| node.parent.is_none());
        if !(placed && aside) {
            continue;
        }
        changes.insert_nodes_before(first, &[*original]);
        for part in parts {
            // The node itself, which nothing reads again.
            let _ = changes.remove_and_drop_node(*part);
        }
    }
}

/// Draw match `at` of `marks` as the current one, and the one before as any other.
fn point_at(doc: &mut BaseDocument, marks: &mut Marks, at: Option<usize>) {
    if marks.doc == 0 || marks.doc != identity(doc) || marks.current == at {
        return;
    }
    let mut changes = doc.mutate();
    for (which, value) in [(marks.current, ""), (at, FOUND_CURRENT)] {
        let Some(found) = which.and_then(|which| marks.found.get(which)) else {
            continue;
        };
        for mark in found {
            if changes.doc.get_node(*mark).is_some() {
                changes.set_attribute(*mark, attribute(FOUND_MARK), value);
            }
        }
    }
    marks.current = at;
}

/// The line `mark` is laid out on in `doc`: its top, from the top of the document, and its
/// height. `None` until the mark is laid out.
fn line_of(doc: &BaseDocument, mark: NodeId) -> Option<(f32, f32)> {
    let words = doc.get_node(mark)?.children.first().copied();
    let root = doc.get_node(inline_root(doc, mark)?)?;
    let layout = &root.element_data()?.inline_layout_data.as_ref()?.layout;
    for line in layout.lines() {
        let ours = line.runs().any(|run| {
            run.clusters().any(|cluster| {
                let id = cluster.first_style().brush.id;
                id == mark || Some(id) == words
            })
        });
        if ours {
            let metrics = line.metrics();
            let boxed = root.final_layout();
            let top = root.absolute_position(0.0, 0.0).y
                + boxed.border.top
                + boxed.padding.top
                + metrics.block_min_coord;
            return Some((top, metrics.block_max_coord - metrics.block_min_coord));
        }
    }
    None
}

/// Bring `mark`, in `handle`'s frame, to the middle of what scrolls it: the frame's document,
/// then the reader around the frame. Whether it was laid out to be found.
fn bring_into_view(handle: &NodeHandle, mark: NodeId) -> bool {
    let frame = handle.node_id();
    let mut doc = handle.doc_mut();
    let Some(window) = doc.get_node(frame).map(|node| node.client_height()) else {
        return true;
    };
    // Where the line shows in the frame once the frame's own document is scrolled to it.
    let shown = {
        let Some(sub) = doc.subdoc_mut(frame) else {
            return true;
        };
        let mut inner = sub.inner_mut();
        let Some((top, height)) = line_of(&inner, mark) else {
            return false;
        };
        let root = inner.root_element().id;
        let x = inner.viewport_scroll().x;
        let middle = f64::from(top) - f64::from(window - height) / 2.0;
        inner.scroll_to(root, x, middle.max(0.0), ScrollBehavior::Instant);
        inner.shell_provider.request_redraw();
        (
            f64::from(top) - inner.viewport_scroll().y,
            f64::from(height),
        )
    };
    if let Some((scroller, y)) = scroller_of(&doc, frame, shown) {
        let x = doc
            .get_node(scroller)
            .map_or(0.0, |node| node.scroll_offset().x);
        doc.scroll_to(scroller, x, y, ScrollBehavior::Instant);
    }
    doc.shell_provider.request_redraw();
    true
}

/// The nearest box around `frame` whose content runs past it, and the offset that puts the line
/// `shown` (its top in the frame, and its height) in its middle. Blitz clamps the offset.
fn scroller_of(
    doc: &BaseDocument,
    frame: NodeId,
    (top, height): (f64, f64),
) -> Option<(NodeId, f64)> {
    let node = doc.get_node(frame)?;
    let mut at = node.layout_parent.get();
    while let Some(id) = at {
        let parent = doc.get_node(id)?;
        let client = f64::from(parent.client_height());
        if f64::from(parent.scroll_height()) > client + 0.5 {
            // Both read through the same ancestors, so their difference is where the frame sits
            // in the scroller's content, whatever it is scrolled to now.
            let offset = f64::from(
                node.absolute_position(0.0, 0.0).y - parent.absolute_position(0.0, 0.0).y,
            );
            return Some((id, offset + top - (client - height) / 2.0));
        }
        at = parent.layout_parent.get();
    }
    None
}

/// The reader's side of a find: its frames, provided to the frames it draws, marked again when
/// what is typed changes or a frame is drawn again, and the current match shown when it moves.
/// Call it among the reader's first hooks.
pub(super) fn use_find(shell: Signal<Shell>) -> Frames {
    let changed = use_signal(|| 0_u64);
    let marked = use_signal(|| 0_u64);
    let frames = use_context_provider(|| Frames {
        inner: Rc::default(),
        changed,
        marked,
    });
    let query = use_memo(move || {
        shell
            .read()
            .find
            .as_ref()
            .map(|finding| finding.find.query.clone())
            // The bar closed: nothing typed, so every mark goes.
            .unwrap_or_default()
    });
    let at = use_memo(move || shell.read().find.as_ref().and_then(Finding::at));
    let refinding = frames.clone();
    use_effect(move || {
        let query = query();
        // Read for its moves alone: a frame drawn again is marked again.
        let _ = changed();
        let frames = refinding.clone();
        spawn(async move {
            let Some(total) = frames.refind(&query).await else {
                return;
            };
            let mut shell = shell;
            let news = shell.peek().find.as_ref().is_some_and(|finding| {
                finding.find.query == query && !(finding.counted && finding.total == total)
            });
            if news && let Some(finding) = shell.write().find.as_mut() {
                finding.found(&query, total);
            }
        });
    });
    let pointing = frames.clone();
    use_effect(move || {
        let at = at();
        // Read for its moves alone: new marks are drawn with the current match among them.
        let _ = marked();
        let frames = pointing.clone();
        spawn(async move { frames.point(at).await });
    });
    frames
}

/// ⌘F: the bar, over the open conversation; with the bar open, its field again, what is typed
/// selected. Nothing while nothing is open, or while a message is being written: `false`, and the
/// key is left for the search it was before.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) -> bool {
    let (reading, finding) = {
        let now = shell.peek();
        (
            now.open.is_some() && now.composing.is_none(),
            now.find.is_some(),
        )
    };
    if !reading {
        return false;
    }
    if finding {
        Host::focus_all(FIELD);
    } else {
        shell.write().find = Some(Finding::default());
    }
    true
}

#[cfg(test)]
mod open_tests {
    use super::open;
    use crate::ui::view::Shell;
    use dioxus::prelude::*;
    use mail_domain::ThreadId;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};

    #[derive(Clone)]
    struct Probe(Option<ThreadId>, Arc<AtomicU8>);

    #[component]
    fn Opener() -> Element {
        let Probe(thread, took) = consume_context::<Probe>();
        let shell = use_signal(|| Shell {
            open: thread,
            ..Shell::default()
        });
        use_hook(|| took.store(1 + u8::from(open(shell)), Ordering::SeqCst));
        rsx! {}
    }

    fn took(thread: Option<ThreadId>) -> bool {
        let took = Arc::new(AtomicU8::new(0));
        let mut dom = VirtualDom::new(Opener).with_root_context(Probe(thread, took.clone()));
        dom.rebuild_in_place();
        took.load(Ordering::SeqCst) == 2
    }

    /// ⌘F finds in the open conversation, and with none open is not taken: it falls through to
    /// the search it was.
    #[test]
    fn find_is_taken_only_by_an_open_conversation() {
        assert!(took(Some(ThreadId::generate())));
        assert!(!took(None));
    }
}

#[cfg(test)]
mod stamp_tests {
    use super::next_stamp;

    /// Each document a find marks gets a stamp of its own, and 0 is left to documents no find has
    /// marked, so a document drawn again never matches the marks of the one it replaced.
    #[test]
    fn every_stamp_is_new_and_none_is_the_unmarked() {
        let (a, b, c) = (next_stamp(), next_stamp(), next_stamp());
        assert!(a != 0 && b != 0 && c != 0);
        assert!(a != b && b != c && a != c);
    }
}

/// Enter or ⌘G, Shift+Enter or ⇧⌘G: the next or previous match.
pub(in crate::ui) fn step(mut shell: Signal<Shell>, step: Step) {
    if shell.peek().find.is_none() {
        return;
    }
    if let Some(finding) = shell.write().find.as_mut() {
        finding.step(step);
    }
}

/// Escape or Done: the bar closes, the marks go, and the keyboard is the window's again.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    if shell.peek().find.is_some() {
        shell.write().find = None;
        Host::focus_app();
    }
}

/// What the bar says beside what is typed: the match it is on, or that there is none; nothing
/// while nothing is typed or it is still counting.
fn count(finding: &Finding) -> Option<String> {
    if finding.find.query.trim().is_empty() || !finding.counted {
        return None;
    }
    Some(match finding.at() {
        Some(at) => copy::status::match_of(at + 1, finding.total),
        None => copy::status::NO_MATCHES.to_owned(),
    })
}

/// ⌘F's bar: the field, with the keyboard as it opens, the count, and the steps and Done.
#[component]
pub(super) fn FindBar(shell: Signal<Shell>) -> Element {
    let keys = use_keys();
    let focus = FieldFocus::Controlled(use_focus_request().with_select_all());
    let Some(finding) = shell.read().find.clone() else {
        return rsx! {};
    };
    let said = count(&finding);
    let any = finding.total > 0 && finding.counted;
    rsx! {
        div { class: "find-bar",
            TextField {
                label: copy::aria::FIND_IN_MESSAGE.to_owned(),
                value: finding.find.query.clone(),
                placeholder: copy::fields::FIND.to_owned(),
                kind: FieldKind::Search,
                size: ControlSize::Small,
                focus,
                common: crate::ui::common::classed("find-field"),
                suffix: said.map(|said| rsx! { span { class: "find-count", "{said}" } }),
                oninput: move |text: String| {
                    if let Some(finding) = shell.write().find.as_mut() {
                        finding.retype(text);
                    }
                },
                onkey: move |event: KeyboardEvent| {
                    // A chord is still the window's: ⌘F selects what is typed, ⌘G steps, ⌘K
                    // searches the mail. Every other key is the field's.
                    if crate::ui::actions::is_chord(keys, &event.key(), event.modifiers()) {
                        return;
                    }
                    event.stop_propagation();
                    match event.key() {
                        Key::Enter => {
                            event.prevent_default();
                            let back = event.modifiers().contains(Modifiers::SHIFT);
                            step(shell, if back { Step::Previous } else { Step::Next });
                        }
                        Key::Escape => close(shell),
                        _ => {}
                    }
                },
            }
            Button {
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                icon: Some(IconSource::Glyph(Icon::ChevronUp)),
                label: copy::aria::PREVIOUS_MATCH.to_owned(),
                title: Some(copy::aria::PREVIOUS_MATCH.to_owned()),
                title_shortcut: Some(crate::ui::actions::tip_standard(StandardAction::FindPrevious)),
                availability: crate::ui::press::available(any),
                onclick: crate::ui::press::on_primary(move || step(shell, Step::Previous)),
            }
            Button {
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                icon: Some(IconSource::Glyph(Icon::ChevronDown)),
                label: copy::aria::NEXT_MATCH.to_owned(),
                title: Some(copy::aria::NEXT_MATCH.to_owned()),
                title_shortcut: Some(crate::ui::actions::tip_standard(StandardAction::FindNext)),
                availability: crate::ui::press::available(any),
                onclick: crate::ui::press::on_primary(move || step(shell, Step::Next)),
            }
            Button {
                label: copy::controls::DONE.to_owned(),
                title_shortcut: Some(crate::ui::actions::escape()),
                onclick: crate::ui::press::on_primary(move || close(shell)),
            }
        }
    }
}
