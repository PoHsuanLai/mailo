//! A signature in a document: where it is, which one it is, and the one edit that puts another
//! in its place.
//!
//! The editor has no container to keep a signature in. What it has is the `-- ` separator, an
//! object ([`Object::Signature`]), with the signature's paragraphs after it, as RFC 3676 lays a
//! text part out. So the signature is read from the last separator to what ends it: the quoted
//! original, a quoted passage, the forwarded block, or the end. When its first nodes say what one
//! of the identity's signatures says, it is that one and only those nodes are it, so words typed
//! under a signature survive its being changed.

use std::ops::Range;

use mail_domain::{Signature, SignatureId};
use mail_mime::{RemoteImages, SanitizePolicy, from_html, sanitize};

use crate::compose::FORWARDED;
use crate::editor::{
    Doc, Node, Object, Op, ParaKind, nodes_from_blocks, nodes_from_plain, runs_text,
};

/// Where a reply's signature goes when it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placement {
    /// Under the quoted original, at the very end, as Apple Mail places it unless asked.
    #[default]
    BelowQuote,
    /// Under what was written and above the quoted original.
    AboveQuote,
}

/// The editor's nodes for `signature`: its HTML when it was written in the composer, read the
/// way a draft's is, else its text.
pub fn nodes_of(signature: &Signature) -> Vec<Node> {
    let nodes = match &signature.html {
        Some(html) => {
            let safe = sanitize(html, SanitizePolicy::CURRENT);
            nodes_from_blocks(&from_html(&safe, &[], RemoteImages::Blocked).blocks)
        }
        None => Vec::new(),
    };
    if nodes.iter().any(|node| !blank(node)) {
        return nodes;
    }
    nodes_from_plain(signature.text.trim_matches(['\r', '\n']))
}

/// `nodes` as plain text: a line a paragraph, a list item with its marker, an object by what it
/// stands for. What a signature written in the composer keeps as its text.
pub fn plain_of(nodes: &[Node]) -> String {
    let lines: Vec<String> = nodes
        .iter()
        .filter_map(|node| match node {
            Node::Para { kind, runs } => {
                let text = runs_text(runs);
                Some(match kind {
                    ParaKind::Bullet => format!("- {text}"),
                    ParaKind::Todo(_) => format!("[ ] {text}"),
                    _ => text,
                })
            }
            Node::Object(Object::Divider) => Some("----".to_owned()),
            Node::Object(Object::Image { alt, .. }) if !alt.is_empty() => Some(alt.clone()),
            Node::Object(_) => None,
        })
        .collect();
    lines.join("\n").trim_matches('\n').to_owned()
}

/// Where the signature is in `doc`: from its separator to what ends it.
pub fn region(doc: &Doc) -> Option<Range<usize>> {
    let start = doc
        .nodes
        .iter()
        .rposition(|node| matches!(node, Node::Object(Object::Signature)))?;
    let end = doc.nodes[start + 1..]
        .iter()
        .position(ends_signature)
        .map_or(doc.nodes.len(), |at| start + 1 + at);
    Some(start..end)
}

/// Which of `known` the signature in `doc` is, and the nodes that are it: the separator and the
/// fewest nodes after it that say what the signature says. `None` when there is no signature, or
/// it has been changed from every one of them.
pub fn recognise(doc: &Doc, known: &[Signature]) -> Option<(SignatureId, Range<usize>)> {
    let found = region(doc)?;
    let after = found.start + 1;
    known
        .iter()
        .filter(|signature| !signature.is_blank())
        .find_map(|signature| {
            let wanted = words(&signature.text);
            (after..=found.end)
                .find(|&end| words(&plain_of(&doc.nodes[after..end])) == wanted)
                .map(|end| (signature.id, found.start..end))
        })
}

/// The edit that makes `doc`'s signature `next`, or takes it off with `None`, and the node the
/// signature then starts at. `None` when there is nothing to do.
///
/// The signature there is replaced: only its own nodes when it is one of `known`, else all of it.
/// With none, `next` goes under the quoted original, or above it for [`Placement::AboveQuote`],
/// and at the end of a message that quotes nothing.
pub fn swap(
    doc: &Doc,
    known: &[Signature],
    next: Option<&Signature>,
    placement: Placement,
) -> Option<(Op, usize)> {
    let put: Vec<Node> = match next.filter(|signature| !signature.is_blank()) {
        Some(signature) => std::iter::once(Node::Object(Object::Signature))
            .chain(nodes_of(signature))
            .collect(),
        None => Vec::new(),
    };
    let at = recognise(doc, known)
        .map(|(_, at)| at)
        .or_else(|| region(doc))
        .or_else(|| {
            let index = match placement {
                Placement::AboveQuote => doc.nodes.iter().position(ends_signature),
                Placement::BelowQuote => None,
            }
            .unwrap_or_else(|| after_last_words(doc));
            (!put.is_empty()).then_some(index..index)
        })?;
    let mut nodes = put;
    if doc.nodes[at.clone()] == nodes[..] {
        return None;
    }
    // A document ends in a paragraph and is never empty: a line to type on after an object.
    let before_ok = at.start > 0 && matches!(doc.nodes[at.start - 1], Node::Para { .. });
    let ends = at.end == doc.nodes.len();
    let last_is_para = match nodes.last() {
        Some(node) => matches!(node, Node::Para { .. }),
        None => before_ok,
    };
    if ends && !last_is_para {
        nodes.push(Node::plain(ParaKind::Paragraph, ""));
    }
    Some((
        Op::Replace {
            index: at.start,
            take: at.len(),
            nodes,
        },
        at.start,
    ))
}

/// Where a signature goes at the end of `doc`: after the last node with something in it, so the
/// empty line a new message opens on stays above it, to type on.
fn after_last_words(doc: &Doc) -> usize {
    doc.nodes
        .iter()
        .rposition(|node| !blank(node))
        .map_or(doc.nodes.len().min(1), |at| at + 1)
}

/// What ends a signature read down from its separator: the quoted original, a quoted passage, or
/// the forwarded block's first line.
fn ends_signature(node: &Node) -> bool {
    match node {
        Node::Object(Object::QuotedMessage { .. }) => true,
        Node::Para {
            kind: ParaKind::Quote,
            ..
        } => true,
        Node::Para { runs, .. } => runs_text(runs).trim() == FORWARDED,
        Node::Object(_) => false,
    }
}

/// Whether a node says nothing: an empty paragraph.
fn blank(node: &Node) -> bool {
    matches!(node, Node::Para { runs, .. } if runs_text(runs).trim().is_empty())
}

/// `text`'s words, each run of white space one space: what two signatures are compared by, since
/// a paragraph break and a line break are the same in a signature.
fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
