//! The document a stored draft opens as: the composer's HTML when it wrote one, else the text,
//! with its `-- ` signature and a quoted original folded into one object.
//!
//! Here rather than in the window because a draft is opened as a document on both sides of it:
//! the composer edits one, and [`crate::signature`] puts a signature into one when From changes,
//! whether or not a composer is open on the draft.

use mail_domain::Draft;
use mail_mime::{
    Block, Delsp, Dir, ParaStyle, ParsedPart, RemoteImages, SanitizePolicy, Span, from_html,
    sanitize, unflow,
};

use mail_editor::{Doc, Node, Object, ParaKind, nodes_from_blocks, nodes_from_plain, runs_text};

/// The document a stored draft holds: its HTML when the composer wrote one, else its text. Its
/// images are found among `inline`, the draft's own pictures; one not there is dropped.
pub fn doc_of(draft: &Draft, inline: &[ParsedPart]) -> Doc {
    let nodes = match &draft.html {
        Some(html) => {
            let safe = sanitize(html, SanitizePolicy::CURRENT);
            separated(nodes_from_blocks(
                &from_html(&safe, inline, RemoteImages::Blocked).blocks,
            ))
        }
        // `Draft.text` is `format=flowed`: its soft breaks are not the author's.
        None => doc_from_text(&unflow(&draft.text, Delsp::No)),
    };
    finish(nodes)
}

/// `nodes` with the signature separator the composer wrote as `<p>-- </p>` an object again: the
/// last paragraph that says only `--`, as RFC 3676 reads the last `-- ` line of a text part.
/// Read back as a paragraph it would be words the person could type over, and a signature no
/// picker could find.
fn separated(mut nodes: Vec<Node>) -> Vec<Node> {
    let separator = nodes.iter().rposition(|node| {
        matches!(node, Node::Para { kind: ParaKind::Paragraph, runs, .. }
            if runs_text(runs).trim() == "--")
    });
    if let Some(at) = separator {
        nodes[at] = Node::Object(Object::Signature);
    }
    nodes
}

/// A document ends in a paragraph, so there is always a line to type on after an object.
fn finish(mut nodes: Vec<Node>) -> Doc {
    if !matches!(nodes.last(), Some(Node::Para { .. })) {
        nodes.push(Node::plain(ParaKind::Paragraph, ""));
    }
    Doc { nodes }
}

/// Plain draft text as nodes: paragraphs, the `-- ` signature, and a quoted original folded
/// into one object.
pub fn doc_from_text(text: &str) -> Vec<Node> {
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut nodes = Vec::new();
    let mut body = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line == "-- " {
            push_body(&mut nodes, &mut body);
            nodes.push(Node::Object(Object::Signature));
            index += 1;
            continue;
        }
        if let Some((who, when)) = attribution(line) {
            let quoted: Vec<&str> = lines[index + 1..]
                .iter()
                .take_while(|line| line.starts_with('>'))
                .copied()
                .collect();
            push_body(&mut nodes, &mut body);
            index += 1 + quoted.len();
            nodes.push(Node::Object(Object::QuotedMessage {
                who,
                when,
                body: quoted_blocks(&quoted),
            }));
            continue;
        }
        body.push(line);
        index += 1;
    }
    push_body(&mut nodes, &mut body);
    if !matches!(nodes.first(), Some(Node::Para { .. })) {
        nodes.insert(0, Node::plain(ParaKind::Paragraph, ""));
    }
    nodes
}

fn push_body(nodes: &mut Vec<Node>, body: &mut Vec<&str>) {
    let text = body.join("\n");
    body.clear();
    if text.trim().is_empty() {
        return;
    }
    nodes.extend(nodes_from_plain(text.trim_matches('\n')));
}

/// `On {when}, {who} wrote:` as `(who, when)`.
fn attribution(line: &str) -> Option<(String, String)> {
    let inner = line.strip_prefix("On ")?.strip_suffix(" wrote:")?;
    let (when, who) = inner.rsplit_once(", ")?;
    Some((who.to_owned(), when.to_owned()))
}

fn quoted_blocks(lines: &[&str]) -> Vec<Block> {
    lines
        .iter()
        .map(|line| line.trim_start_matches('>').trim_start())
        .filter(|line| !line.is_empty())
        .map(|line| Block::Paragraph {
            spans: vec![Span::Text(line.to_owned())],
            dir: Dir::Auto,
            style: ParaStyle::default(),
        })
        .collect()
}
