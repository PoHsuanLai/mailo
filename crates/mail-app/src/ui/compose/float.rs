//! The `/` menu, the `@` menu, Turn into and the ⋮⋮ object menu: when they open, what they
//! list, and what a choice does. Every choice is an `editor::` op; nothing here edits text.

use unicode_segmentation::UnicodeSegmentation;

pub(in crate::ui) use super::items::{
    hue, mention_items, object_items, people_rows, slash_items, turn_items,
};
use super::page::{CcRow, Float, Page};
use crate::editor::{
    Action, Caret, ImageRef, InputEvent, Node, Object, Op, ParaKind, Pos, Range, Table, apply_all,
    catalog, joins_cc, node_len, runs_text, turn_into,
};

/// The longest query a `/` or `@` menu follows before it gives up and closes.
const LONGEST_QUERY: usize = 24;

/// Open, follow or close the `/` and `@` menus after an edit.
pub(in crate::ui) fn after_input(page: &mut Page, event: &InputEvent) {
    match page.float {
        Float::Slash { .. } | Float::Mention { .. } => follow(page),
        _ => {
            let opens = event.input_type == "insertText"
                && matches!(event.data.as_deref(), Some("/") | Some("@"));
            if opens {
                open(page, event.data.as_deref() == Some("/"));
            }
        }
    }
}

/// The caret moved without typing. A menu whose query the caret has left closes.
pub(in crate::ui) fn after_move(page: &mut Page) {
    if matches!(page.float, Float::Slash { .. } | Float::Mention { .. }) {
        follow(page);
    }
    if page.selection.is_none() && matches!(page.float, Float::Turn | Float::Link(_)) {
        page.float = Float::Closed;
    }
}

fn open(page: &mut Page, slash: bool) {
    let caret = page.session.caret.pos;
    let Some(anchor) = caret
        .offset
        .checked_sub(1)
        .map(|at| Pos::new(caret.node, at))
    else {
        return;
    };
    let text = paragraph_text(page, caret.node);
    let before: Vec<&str> = text.graphemes(true).take(anchor.offset).collect();
    let starts_a_word = before
        .last()
        .is_none_or(|grapheme| grapheme.chars().all(char::is_whitespace));
    if starts_a_word {
        page.float = if slash {
            Float::Slash { anchor, active: 0 }
        } else {
            Float::Mention { anchor, active: 0 }
        };
    }
}

fn follow(page: &mut Page) {
    if query(page).is_none() {
        page.float = Float::Closed;
    } else if let Float::Slash { active, .. } | Float::Mention { active, .. } = &mut page.float {
        *active = 0;
    }
}

fn paragraph_text(page: &Page, node: usize) -> String {
    match page.session.doc.nodes.get(node) {
        Some(Node::Para { runs, .. }) => runs_text(runs),
        _ => String::new(),
    }
}

/// What follows the `/` or `@`, up to the caret. `None` when the caret has left it.
pub(in crate::ui) fn query(page: &Page) -> Option<String> {
    let anchor = match page.float {
        Float::Slash { anchor, .. } | Float::Mention { anchor, .. } => anchor,
        _ => return None,
    };
    let caret = page.session.caret.pos;
    if caret.node != anchor.node || caret.offset <= anchor.offset {
        return None;
    }
    let text = paragraph_text(page, anchor.node);
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let marker = graphemes.get(anchor.offset)?;
    if *marker != "/" && *marker != "@" {
        return None;
    }
    let typed: String = graphemes.get(anchor.offset + 1..caret.offset)?.concat();
    let too_long = typed.graphemes(true).count() > LONGEST_QUERY;
    (!too_long && !typed.contains("  ")).then_some(typed)
}

/// While the `@` menu is open, ask the contact book for what follows the `@`: the same
/// question, and so the same people in the same order, as the To and Cc fields. The whole book's
/// top when nothing follows it yet.
pub(in crate::ui) fn suggest_mention(page: &mut Page, store: &dyn mail_store::Store) {
    if !matches!(page.float, Float::Mention { .. }) {
        return;
    }
    if let Some(typed) = query(page) {
        page.people = super::super::contacts::book::suggest(store, &typed);
    }
}

/// What a `/` choice asks of the page beyond the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Picked {
    /// Done: the document changed, or nothing matched.
    Done,
    /// Attach a file: the page shows its file picker.
    Attach,
}

/// Apply `ops` as one undo step, with the caret after them.
pub(in crate::ui) fn commit(page: &mut Page, ops: Vec<Op>, caret: Caret) -> bool {
    let mut doc = page.session.doc.clone();
    let Ok(inverses) = apply_all(&mut doc, ops.clone()) else {
        return false;
    };
    page.session.doc = doc;
    page.session.log.record_structural(ops, inverses);
    page.session.caret = caret;
    page.session.pending = None;
    page.selection = None;
    page.touch();
    true
}

fn typed_span(page: &Page) -> Option<Range> {
    let anchor = match page.float {
        Float::Slash { anchor, .. } | Float::Mention { anchor, .. } => anchor,
        _ => return None,
    };
    Some(Range {
        start: anchor,
        end: page.session.caret.pos,
    })
}

/// The `/` item named `key`. `today` is the date as the page writes it: this has no clock.
pub(in crate::ui) fn pick_slash(page: &mut Page, key: &str, today: &str) -> Picked {
    let Some(item) = catalog().iter().find(|item| item.name == key) else {
        return Picked::Done;
    };
    let Some(typed) = typed_span(page) else {
        return Picked::Done;
    };
    page.float = Float::Closed;
    let at = typed.start;
    let mut ops = vec![Op::Delete { range: typed }];
    let insert = |text: &str| Op::Insert {
        at,
        text: text.to_owned(),
        marks: crate::editor::Marks::new(),
    };
    let caret = match item.action {
        Action::Turn(kind) => {
            ops.push(Op::SetKind {
                range: Range { start: at, end: at },
                kind,
            });
            Caret::at(at.node, at.offset)
        }
        Action::Snippet => {
            ops.push(insert(crate::editor::SNIPPET));
            Caret::at(
                at.node,
                at.offset + crate::editor::grapheme_len(crate::editor::SNIPPET),
            )
        }
        Action::Date => {
            ops.push(insert(today));
            Caret::at(at.node, at.offset + crate::editor::grapheme_len(today))
        }
        Action::Attachment => {
            commit(page, ops, Caret::at(at.node, at.offset));
            return Picked::Attach;
        }
        Action::Divider => return insert_object(page, ops, at, Object::Divider),
        Action::Image => {
            let image = Object::Image {
                src: ImageRef::new(""),
                alt: String::new(),
            };
            return insert_object(page, ops, at, image);
        }
        Action::Table => {
            let table = Table {
                rows: vec![
                    vec!["Item".to_owned(), "Detail".to_owned()],
                    vec![String::new(), String::new()],
                    vec![String::new(), String::new()],
                ],
            };
            return insert_object(page, ops, at, Object::Table(table));
        }
        Action::Signature => return insert_object(page, ops, at, Object::Signature),
    };
    commit(page, ops, caret);
    Picked::Done
}

/// Put `object` where the `/` was: in place of an empty line, else beside or inside the text.
/// The caret lands on the line after it, which exists because an empty one is added when needed.
fn insert_object(page: &mut Page, mut ops: Vec<Op>, at: Pos, object: Object) -> Picked {
    let mut after = page.session.doc.clone();
    if apply_all(&mut after, ops.clone()).is_err() {
        return Picked::Done;
    }
    let len = after.nodes.get(at.node).map(node_len).unwrap_or(0);
    let blank = || Node::plain(ParaKind::Paragraph, "");
    let followed = matches!(after.nodes.get(at.node + 1), Some(Node::Para { .. }));
    let (nodes, caret) = if len == 0 {
        (
            vec![Node::Object(object), blank()],
            Pos::new(at.node + 1, 0),
        )
    } else if at.offset == 0 {
        (vec![Node::Object(object)], Pos::new(at.node + 1, 0))
    } else if at.offset == len && !followed {
        (
            vec![Node::Object(object), blank()],
            Pos::new(at.node + 2, 0),
        )
    } else {
        (vec![Node::Object(object)], Pos::new(at.node + 2, 0))
    };
    ops.push(Op::InsertNodes { at, nodes });
    commit(page, ops, Caret::at(caret.node, caret.offset));
    Picked::Done
}

/// `@name` becomes the name, and the person joins Cc unless they are already on the message.
pub(in crate::ui) fn pick_mention(page: &mut Page, address: &str) {
    let Some(person) = page
        .people
        .iter()
        .find(|person| person.address == address)
        .cloned()
    else {
        return;
    };
    let Some(typed) = typed_span(page) else {
        return;
    };
    page.float = Float::Closed;
    let text = format!("@{} ", person.name);
    let caret = Caret::at(
        typed.start.node,
        typed.start.offset + crate::editor::grapheme_len(&text),
    );
    let ops = vec![
        Op::Delete { range: typed },
        Op::Insert {
            at: typed.start,
            text,
            marks: crate::editor::Marks::new(),
        },
    ];
    if !commit(page, ops, caret) {
        return;
    }
    if let Some(joining) = joins_cc(&person, &page.to, &page.cc).cloned() {
        page.flash = Some(joining.address.clone());
        page.cc.push(joining);
        page.cc_row = CcRow::Shown;
    }
}

/// Turn every paragraph the selection touches, or the caret's, into `key`'s kind.
pub(in crate::ui) fn pick_turn(page: &mut Page, key: &str) {
    let Some(Action::Turn(kind)) = turn_into()
        .into_iter()
        .find(|item| item.name == key)
        .map(|item| item.action)
    else {
        return;
    };
    let caret = page.session.caret;
    let range = page.selection.unwrap_or(Range {
        start: caret.pos,
        end: caret.pos,
    });
    page.float = Float::Closed;
    commit(page, vec![Op::SetKind { range, kind }], caret);
}

/// The ⋮⋮ menu's choice for the object at `index`.
pub(in crate::ui) fn pick_object(page: &mut Page, index: usize, key: &str) {
    page.float = Float::Closed;
    let nodes = &page.session.doc.nodes;
    let Some(node) = nodes.get(index).cloned() else {
        return;
    };
    let caret = page.session.caret;
    let ops = match key {
        "up" if index > 0 => vec![Op::Replace {
            index: index - 1,
            take: 2,
            nodes: vec![node, nodes[index - 1].clone()],
        }],
        "down" if index + 1 < nodes.len() => vec![Op::Replace {
            index,
            take: 2,
            nodes: vec![nodes[index + 1].clone(), node],
        }],
        "dup" => vec![Op::Replace {
            index,
            take: 1,
            nodes: vec![node.clone(), node],
        }],
        "del" => vec![Op::Delete {
            range: Range {
                start: Pos::new(index, 0),
                end: Pos::new(index, 1),
            },
        }],
        _ => return,
    };
    let caret = match key {
        "del" => Caret::at(index.min(nodes.len().saturating_sub(2)), 0),
        _ => caret,
    };
    commit(page, ops, caret);
}

/// The kind of the paragraph the caret is in.
pub(in crate::ui) fn current_kind(page: &Page) -> Option<ParaKind> {
    match page.session.doc.nodes.get(page.session.caret.pos.node) {
        Some(Node::Para { kind, .. }) => Some(*kind),
        _ => None,
    }
}
