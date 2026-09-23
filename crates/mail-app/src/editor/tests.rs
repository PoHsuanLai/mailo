//! Behaviour the composer has to keep. The two properties live here; the tables are in
//! `tests/`, one file per concern.
//!
//! This file keeps its name because `proptest-regressions/editor/tests.txt` is keyed to it:
//! the seeds recorded there are replayed before any new case.

mod events;
mod keys;
mod markdown;
mod menus;
mod strategies;
mod writers;

use mail_mime::{RemoteImages, SafeUrl, SanitizePolicy, from_html, sanitize};
use proptest::prelude::*;

use super::*;
use strategies::{doc_and_ops, marks_strategy};

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
    SafeUrl::parse("https://example.com/a").expect("a fixed https URL parses")
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
/// a button, which keeps the label and the URL and drops the styles. Neighbouring anchors to
/// the same URL count as one link.
fn buttonized(kind: ParaKind, runs: &[Run]) -> bool {
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
    runs: Vec<(String, u8, Option<String>)>,
}

/// What `from_html(to_html(doc))` is expected to preserve, and nothing more:
/// - objects are left out (the writer and the reader disagree on them by design);
/// - an empty paragraph is left out, since `from_html` drops it;
/// - underline and strike are dropped, since `Span` has neither;
/// - code keeps its text only; a lone short link keeps its label and URL only;
/// - space at either end of a paragraph is dropped, as HTML does.
fn canon(doc: &Doc) -> Vec<Canon> {
    doc.nodes
        .iter()
        .filter_map(|node| match node {
            Node::Para { kind, runs } => canon_para(*kind, runs),
            Node::Object(_) => None,
        })
        .collect()
}

fn canon_para(kind: ParaKind, runs: &[Run]) -> Option<Canon> {
    let styled = !matches!(kind, ParaKind::Code) && !buttonized(kind, runs);
    let mut kept: Vec<(String, u8, Option<String>)> = Vec::new();
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
        match kept.last_mut() {
            Some((text, prev_bits, prev_link)) if *prev_bits == bits && *prev_link == link => {
                text.push_str(&run.text);
            }
            _ => kept.push((run.text.clone(), bits, link)),
        }
    }
    if let Some((text, ..)) = kept.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some((text, ..)) = kept.last_mut() {
        *text = text.trim_end().to_owned();
    }
    kept.retain(|(text, ..)| !text.is_empty());
    (!kept.is_empty()).then(|| Canon {
        kind: kind_name(kind),
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
    let para = (kind, prop::collection::vec(run, 1..5)).prop_map(|(kind, words)| {
        let last = words.len() - 1;
        let runs = words
            .into_iter()
            .enumerate()
            .map(|(index, (word, marks))| {
                let space = if index == last { "" } else { " " };
                Run::new(format!("{word}{space}"), marks)
            })
            .collect();
        Node::para(kind, runs)
    });
    prop::collection::vec(para, 1..6).prop_map(|nodes| Doc { nodes })
}

/// The document the recorded seed (`n = 2`) was drawn from: a numbered item whose whole
/// text is one bold, italic link, which `from_html` makes a button.
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
        prop_assert!(!html.contains("style="), "{}", html);
        prop_assert_eq!(canon(&round_trip(&doc)), canon(&doc), "{}", html);
    }
}

#[test]
fn the_recorded_seed_is_a_lone_link_that_becomes_a_button() {
    let doc = sample_marked_doc(2);
    let Some(Node::Para { kind, runs }) = doc.nodes.last() else {
        panic!("the sample ends in a paragraph");
    };
    assert_eq!(*kind, ParaKind::Numbered);
    assert!(buttonized(*kind, runs), "{runs:?}");
    let back = round_trip(&doc);
    let Some(Node::Para { kind, runs }) = back.nodes.last() else {
        panic!("the round trip ends in a paragraph: {back:?}");
    };
    assert_eq!(*kind, ParaKind::Numbered, "the item stays an item");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "code");
    assert_eq!(runs[0].marks.link, Some(url()), "the link survives");
    assert!(!runs[0].marks.has(Mark::Bold), "the button drops the bold");
    assert_eq!(canon(&back), canon(&doc));
}
