//! Random documents and random edits for the inverse property.

use proptest::prelude::*;

use super::*;

/// Clusters that stay themselves next to any other: no bare combining mark, no lone
/// regional indicator, no CR. Offsets are counted run by run, so a character that fuses
/// with its neighbour into one cluster across an edit boundary is outside the model.
const CLUSTERS: &[&str] = &[
    "a", "b", "Z", " ", "字", "語", "😀", FAMILY, "é", "e\u{301}", "🇹🇼", "\n", ">", "-",
];

fn text_strategy(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(CLUSTERS), 1..=max).prop_map(|parts| parts.concat())
}

pub(super) fn marks_strategy() -> impl Strategy<Value = Marks> {
    (0u8..32, any::<bool>()).prop_map(|(bits, linked)| {
        let mut marks = Marks::new();
        for (index, mark) in [
            Mark::Bold,
            Mark::Italic,
            Mark::Underline,
            Mark::Strike,
            Mark::Code,
        ]
        .into_iter()
        .enumerate()
        {
            if bits & (1 << index) != 0 {
                marks.set(mark, Presence::On);
            }
        }
        if linked {
            marks.link = Some(url());
        }
        marks
    })
}

fn kind_strategy() -> impl Strategy<Value = ParaKind> {
    prop::sample::select(vec![
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
    ])
}

fn object_strategy() -> impl Strategy<Value = Object> {
    prop_oneof![
        Just(Object::Divider),
        Just(Object::Signature),
        Just(Object::Attachment(AttachmentRef::new("a.pdf"))),
        Just(Object::Image {
            src: ImageRef::new("cid:1"),
            alt: "a cat".into(),
        }),
        Just(Object::Table(Table {
            rows: vec![vec!["a".into(), "b".into()]],
        })),
    ]
}

fn node_strategy() -> impl Strategy<Value = Node> {
    prop_oneof![
        4 => (
            kind_strategy(),
            prop::collection::vec((text_strategy(4), marks_strategy()), 0..4),
        )
            .prop_map(|(kind, runs)| {
                Node::para(
                    kind,
                    runs.into_iter().map(|(text, marks)| Run::new(text, marks)).collect(),
                )
            }),
        1 => object_strategy().prop_map(Node::Object),
    ]
}

fn doc_strategy() -> impl Strategy<Value = Doc> {
    prop::collection::vec(node_strategy(), 1..6).prop_map(|nodes| Doc { nodes })
}

/// A position in `doc`: mostly inside it, sometimes one past a node's end or the last node.
fn pos_strategy(doc: &Doc) -> impl Strategy<Value = Pos> + use<> {
    let lens: Vec<usize> = doc.nodes.iter().map(node_len).collect();
    (0..=lens.len(), any::<prop::sample::Index>()).prop_map(move |(node, offset)| {
        let len = lens.get(node).copied().unwrap_or(0);
        Pos::new(node, offset.index(len + 2))
    })
}

fn range_strategy(doc: &Doc) -> impl Strategy<Value = Range> + use<> {
    (pos_strategy(doc), pos_strategy(doc)).prop_map(|(start, end)| Range { start, end })
}

fn op_strategy(doc: &Doc) -> impl Strategy<Value = Op> + use<> {
    let count = doc.nodes.len();
    prop_oneof![
        1 => (pos_strategy(doc), text_strategy(3), marks_strategy())
            .prop_map(|(at, text, marks)| Op::Insert { at, text, marks }),
        2 => range_strategy(doc).prop_map(|range| Op::Delete { range }),
        1 => pos_strategy(doc).prop_map(|at| Op::Split { at }),
        1 => (0..=count).prop_map(|node| Op::Merge { node }),
        1 => (range_strategy(doc), kind_strategy()).prop_map(|(range, kind)| Op::SetKind { range, kind }),
        1 => (
            range_strategy(doc),
            prop::sample::select(vec![Mark::Bold, Mark::Italic, Mark::Code]),
            prop::sample::select(vec![Presence::On, Presence::Off]),
        )
            .prop_map(|(range, mark, on)| Op::SetMark { range, mark, on }),
        1 => (range_strategy(doc), any::<bool>()).prop_map(|(range, linked)| Op::SetLink {
            range,
            url: linked.then(url),
        }),
        1 => (pos_strategy(doc), prop::collection::vec(node_strategy(), 0..3))
            .prop_map(|(at, nodes)| Op::InsertNodes { at, nodes }),
        1 => (0..=count, 0usize..3, prop::collection::vec(node_strategy(), 0..3))
            .prop_map(|(index, take, nodes)| Op::Replace { index, take, nodes }),
    ]
}

pub(super) fn doc_and_ops(max: usize) -> impl Strategy<Value = (Doc, Vec<Op>)> {
    doc_strategy().prop_flat_map(move |doc| {
        let ops = prop::collection::vec(op_strategy(&doc), 1..=max);
        (Just(doc), ops)
    })
}
