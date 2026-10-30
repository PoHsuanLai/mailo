//! Behaviour the composer has to keep. The two properties live here; the tables are in
//! `tests/`, one file per concern.
//!
//! This file keeps its name because `proptest-regressions/editor/tests.txt` is keyed to it:
//! the seeds recorded there are replayed before any new case.

mod events;
mod format;
mod keys;
mod markdown;
mod menus;
mod strategies;
mod writers;

use mail_mime::{RemoteImages, SafeUrl, SanitizePolicy, from_html, sanitize};
use proptest::prelude::*;

use super::*;
use strategies::{doc_and_ops, layout_strategy, marks_strategy};

const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";

fn plain(kind: ParaKind, text: &str) -> Node {
    Node::plain(kind, text)
}

fn doc_of(nodes: Vec<Node>) -> Doc {
    Doc { nodes }
}

/// Each node's text, `|` between nodes, `#` for an object.
fn body(doc: &Doc) -> String {
    doc.nodes
        .iter()
        .map(|node| match node {
            Node::Para { runs, .. } => runs_text(runs),
            Node::Object(_) => "#".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn kind_name(kind: ParaKind) -> &'static str {
    match kind {
        ParaKind::Paragraph => "p",
        ParaKind::Heading(Level::One) => "h1",
        ParaKind::Heading(Level::Two) => "h2",
        ParaKind::Heading(Level::Three) => "h3",
        ParaKind::Bullet => "ul",
        ParaKind::Numbered => "ol",
        ParaKind::Todo(Check::Open) => "todo",
        ParaKind::Todo(Check::Done) => "done",
        ParaKind::Quote => "quote",
        ParaKind::Code => "code",
    }
}

/// Each node's kind, `|` between nodes; `hr` for a divider, `obj` for another object.
fn kinds(doc: &Doc) -> String {
    doc.nodes
        .iter()
        .map(|node| match node {
            Node::Para { kind, .. } => kind_name(*kind),
            Node::Object(Object::Divider) => "hr",
            Node::Object(_) => "obj",
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// Each paragraph's kind and depth (`ul1` is a bullet one level in), `|` between nodes; `hr`
/// and `obj` for objects.
fn levels(doc: &Doc) -> String {
    doc.nodes
        .iter()
        .map(|node| match node {
            Node::Para { kind, layout, .. } => format!("{}{}", kind_name(*kind), layout.depth),
            Node::Object(Object::Divider) => "hr".to_owned(),
            Node::Object(_) => "obj".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// A paragraph of `kind` holding `text`, `depth` deep.
fn deep(kind: ParaKind, text: &str, depth: u8) -> Node {
    Node::laid(
        kind,
        vec![Run::new(text, Marks::new())],
        Layout {
            depth,
            ..Layout::default()
        },
    )
}

fn apply_ops(doc: &mut Doc, ops: Vec<Op>) {
    apply_all(doc, ops).unwrap_or_else(|err| panic!("apply: {err}"));
}

fn caret_event(input_type: &str, data: Option<&str>, caret: Pos, composing: bool) -> InputEvent {
    InputEvent::new(
        input_type,
        data.map(str::to_owned),
        vec![Range {
            start: caret,
            end: caret,
        }],
        composing,
    )
}

fn url() -> SafeUrl {
    "https://example.com/a"
        .parse()
        .expect("a fixed https URL parses")
}

fn marks_of(list: &[Mark]) -> Marks {
    let mut marks = Marks::new();
    for mark in list {
        marks.set(*mark, Presence::On);
    }
    marks
}

// ---------------------------------------------------------------------------------------
// Apply, then the inverse, is the identity.
// ---------------------------------------------------------------------------------------

proptest! {
    /// Every op on a random document: `apply` then `apply(inverse)` is the identity, the
    /// inverse's own inverse redoes the op, and a refused op leaves the document alone.
    #[test]
    fn apply_then_inverse_is_identity((doc, ops) in doc_and_ops(1)) {
        let op = ops[0].clone();
        let mut next = doc.clone();
        match apply(&mut next, op.clone()) {
            Err(_) => prop_assert_eq!(&next, &doc),
            Ok(inverse) => {
                let done = next.clone();
                let redo = apply(&mut next, inverse.clone())
                    .unwrap_or_else(|err| panic!("inverse {inverse:?} of {op:?}: {err}"));
                prop_assert_eq!(&next, &doc, "op {:?}, inverse {:?}", op, inverse);
                apply(&mut next, redo.clone())
                    .unwrap_or_else(|err| panic!("redo {redo:?} of {op:?}: {err}"));
                prop_assert_eq!(&next, &done, "redo {:?}", redo);
            }
        }
    }

    /// A run of ops, undone by their inverses in reverse order, restores the document.
    #[test]
    fn a_sequence_undoes_in_reverse((doc, ops) in doc_and_ops(6)) {
        let mut next = doc.clone();
        let mut inverses = Vec::new();
        for op in ops {
            // `Replace` is the raw splice inverses are made of, and may empty the document.
            // Every edit a user can cause leaves somewhere to type.
            let user_edit = !matches!(op, Op::Replace { .. }) && !next.nodes.is_empty();
            if let Ok(inverse) = apply(&mut next, op) {
                inverses.push(inverse);
                prop_assert!(!user_edit || !next.nodes.is_empty(), "an edit left nowhere to type");
            }
        }
        for inverse in inverses.into_iter().rev() {
            apply(&mut next, inverse.clone())
                .unwrap_or_else(|err| panic!("inverse {inverse:?}: {err}"));
        }
        prop_assert_eq!(next, doc);
    }
}

// ---------------------------------------------------------------------------------------
// `from_html(to_html(doc))` against the doc, through a normalizer.
// ---------------------------------------------------------------------------------------

fn round_trip(doc: &Doc) -> Doc {
    let safe = sanitize(&to_html(doc), SanitizePolicy::CURRENT);
    let parsed = from_html(&safe, &[], RemoteImages::Blocked);
    Doc {
        nodes: nodes_from_blocks(&parsed.blocks),
    }
}

/// `from_html` turns a paragraph (or list item, or quote) whose text is one short link into
/// a call-to-action link, which keeps the label and the URL and drops the styles. Neighbouring
/// anchors to the same URL count as one link.
fn is_call_to_action(kind: ParaKind, runs: &[Run]) -> bool {
    if matches!(
        kind,
        ParaKind::Heading(_) | ParaKind::Code | ParaKind::Todo(_)
    ) {
        return false;
    }
    let mut link = None;
    let mut label = String::new();
    for run in runs {
        match &run.marks.link {
            Some(url) if link.is_none_or(|first| first == url) => {
                link = Some(url);
                label.push_str(&run.text);
            }
            Some(_) => return false,
            None if run.text.trim().is_empty() => {}
            None => return false,
        }
    }
    link.is_some() && (1..=5).contains(&label.split_whitespace().count())
}

/// What survives the round trip, in a form that compares.
#[derive(Debug, PartialEq, Eq)]
struct Canon {
    kind: &'static str,
    align: Align,
    depth: u8,
    runs: Vec<CanonRun>,
}

/// A run's text, its bold, italic and code as bits, its link, size and colour.
#[derive(Debug, PartialEq, Eq)]
struct CanonRun {
    text: String,
    bits: u8,
    link: Option<String>,
    size: Option<TextSize>,
    color: Option<TextColor>,
}

impl CanonRun {
    fn same_look(&self, other: &CanonRun) -> bool {
        (self.bits, &self.link, self.size, self.color)
            == (other.bits, &other.link, other.size, other.color)
    }
}

/// What `from_html(to_html(doc))` is expected to preserve, and nothing more:
/// - objects are left out (the writer and the reader disagree on them by design);
/// - an empty paragraph is left out, since `from_html` drops it;
/// - underline and strike are dropped, since `Span` has neither;
/// - code keeps its text only; a lone short link keeps its label and URL only, and its depth
///   only in a list or a quote;
/// - space at either end of a paragraph is dropped, as HTML does;
/// - a list item is no deeper than one below the item before it, and the first of a list is at
///   the outer level, since a list nests inside an item.
fn canon(doc: &Doc) -> Vec<Canon> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < doc.nodes.len() {
        let Node::Para { kind, runs, layout } = &doc.nodes[index] else {
            index += 1;
            continue;
        };
        if is_item(*kind) {
            // One run of items, as the writer groups them: until a non-item, or an item back at
            // the outer level that is another list.
            let began = std::mem::discriminant(kind);
            let mut before: Option<u8> = None;
            while let Some(Node::Para { kind, runs, layout }) = doc.nodes.get(index) {
                let goes_on = is_item(*kind)
                    && (before.is_none()
                        || layout.depth > 0
                        || std::mem::discriminant(kind) == began);
                if !goes_on {
                    break;
                }
                let depth = layout.depth.min(before.map_or(0, |depth| depth + 1));
                before = Some(depth);
                out.extend(canon_para(*kind, runs, Layout { depth, ..*layout }));
                index += 1;
            }
            continue;
        }
        out.extend(canon_para(*kind, runs, *layout));
        index += 1;
    }
    out
}

fn canon_para(kind: ParaKind, runs: &[Run], layout: Layout) -> Option<Canon> {
    let call = is_call_to_action(kind, runs);
    let styled = !matches!(kind, ParaKind::Code) && !call;
    let mut kept: Vec<CanonRun> = Vec::new();
    for run in runs.iter().filter(|run| !run.text.is_empty()) {
        let bits = if styled {
            [Mark::Bold, Mark::Italic, Mark::Code]
                .into_iter()
                .enumerate()
                .filter(|(_, mark)| run.marks.has(*mark))
                .fold(0u8, |bits, (index, _)| bits | (1 << index))
        } else {
            0
        };
        let link = match kind {
            ParaKind::Code => None,
            _ => run.marks.link.as_ref().map(|url| url.as_str().to_owned()),
        };
        let next = CanonRun {
            text: run.text.clone(),
            bits,
            link,
            size: run.marks.size.filter(|_| styled),
            color: run.marks.color.filter(|_| styled),
        };
        match kept.last_mut() {
            Some(last) if last.same_look(&next) => last.text.push_str(&next.text),
            _ => kept.push(next),
        }
    }
    if let Some(first) = kept.first_mut() {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(last) = kept.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }
    kept.retain(|run| !run.text.is_empty());
    let (align, depth) = match kind {
        ParaKind::Code => (Align::Left, 0),
        // A call-to-action link keeps only its label and link: a paragraph's indent goes with its
        // alignment, and an item's or a quote's depth is the lists or quotes around it.
        ParaKind::Paragraph if call => (Align::Left, 0),
        _ if call => (Align::Left, layout.depth),
        _ => (layout.align, layout.depth),
    };
    (!kept.is_empty()).then(|| Canon {
        kind: kind_name(kind),
        align,
        depth,
        runs: kept,
    })
}

/// A document of the kinds the HTML writer and `from_html` share: words, single spaces,
/// every mark, and text that needs escaping.
fn html_doc_strategy() -> impl Strategy<Value = Doc> {
    let word = prop::sample::select(vec!["Hello", "world", "字甲", "a<b&c>\"'", "code", "x"]);
    let run = (word, marks_strategy());
    let kind = prop::sample::select(vec![
        ParaKind::Paragraph,
        ParaKind::Heading(Level::One),
        ParaKind::Heading(Level::Two),
        ParaKind::Heading(Level::Three),
        ParaKind::Bullet,
        ParaKind::Numbered,
        ParaKind::Todo(Check::Open),
        ParaKind::Todo(Check::Done),
        ParaKind::Quote,
        ParaKind::Code,
    ]);
    let para = (kind, prop::collection::vec(run, 1..5), layout_strategy()).prop_map(
        |(kind, words, layout)| {
            let last = words.len() - 1;
            let runs = words
                .into_iter()
                .enumerate()
                .map(|(index, (word, marks))| {
                    let space = if index == last { "" } else { " " };
                    Run::new(format!("{word}{space}"), marks)
                })
                .collect();
            Node::laid(kind, runs, layout)
        },
    );
    prop::collection::vec(para, 1..6).prop_map(|nodes| Doc { nodes })
}

/// The document the recorded seed (`n = 2`) was drawn from: a numbered item whose whole
/// text is one bold, italic link, which `from_html` makes a call-to-action link.
fn sample_marked_doc(n: usize) -> Doc {
    let words = ["Hello", "world", "字甲", "a b", "code"];
    let count = 1 + (n % 4);
    let mut nodes = Vec::new();
    for index in 0..count {
        let text = words[(n + index) % words.len()];
        let mut marks = Marks::new();
        if n % 3 == index % 3 {
            marks.set(Mark::Bold, Presence::On);
        }
        if (n + index).is_multiple_of(2) {
            marks.set(Mark::Italic, Presence::On);
        }
        if (n + index).is_multiple_of(5) {
            marks.set(Mark::Underline, Presence::On);
            marks.set(Mark::Strike, Presence::On);
        }
        if (n + index).is_multiple_of(4) {
            marks.link = Some(url());
        }
        let kind = match (n + index) % 6 {
            0 => ParaKind::Paragraph,
            1 => ParaKind::Heading(Level::One),
            2 => ParaKind::Heading(Level::Two),
            3 => ParaKind::Bullet,
            4 => ParaKind::Numbered,
            _ => ParaKind::Quote,
        };
        let runs = if index.is_multiple_of(2) {
            vec![Run::new(text, marks)]
        } else {
            vec![Run::new("say ", Marks::new()), Run::new(text, marks)]
        };
        nodes.push(Node::para(kind, runs));
        if (n + index).is_multiple_of(7) {
            nodes.push(plain(ParaKind::Code, "a < b"));
        }
    }
    Doc { nodes }
}

proptest! {
    /// Kept with its original strategy so the seed in `proptest-regressions` replays.
    #[test]
    fn html_writer_round_trip(n in 0usize..48) {
        let doc = sample_marked_doc(n);
        prop_assert!(!to_html(&doc).contains("style="));
        prop_assert_eq!(canon(&round_trip(&doc)), canon(&doc));
    }

    /// Random paragraphs, headings, lists, to-dos, quotes, code and marks.
    #[test]
    fn html_round_trip_random(doc in html_doc_strategy()) {
        let html = to_html(&doc);
        prop_assert_eq!(canon(&round_trip(&doc)), canon(&doc), "{}", html);
    }
}

#[test]
fn the_recorded_seed_is_a_lone_link_that_becomes_a_call_to_action_link() {
    let doc = sample_marked_doc(2);
    let Some(Node::Para { kind, runs, .. }) = doc.nodes.last() else {
        panic!("the sample ends in a paragraph");
    };
    assert_eq!(*kind, ParaKind::Numbered);
    assert!(is_call_to_action(*kind, runs), "{runs:?}");
    let back = round_trip(&doc);
    let Some(Node::Para { kind, runs, .. }) = back.nodes.last() else {
        panic!("the round trip ends in a paragraph: {back:?}");
    };
    assert_eq!(*kind, ParaKind::Numbered, "the item stays an item");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "code");
    assert_eq!(runs[0].marks.link, Some(url()), "the link survives");
    assert!(
        !runs[0].marks.has(Mark::Bold),
        "the call-to-action link drops the bold"
    );
    assert_eq!(canon(&back), canon(&doc));
}
