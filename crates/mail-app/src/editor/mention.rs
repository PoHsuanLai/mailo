//! `@name` against a list of people, and the guard that a message mentions a file it lacks.
//!
//! The detector is a guard, not a judge. "I didn't attach it" still counts: the word is there.

use crate::editor::doc::{Doc, Node, Object};
use crate::editor::text::runs_text;

/// Someone the composer can add to Cc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// The name shown on the chip.
    pub name: String,
    /// The mailbox.
    pub address: String,
}

/// People whose name, then address, starts with `query`.
///
/// The `@` is optional. An empty query is the whole list. Matching is on the token, so
/// `ann` finds Anna and does not find Joanna.
pub fn resolve<'a>(query: &str, people: &'a [Person]) -> Vec<&'a Person> {
    let query = query.trim().trim_start_matches('@').to_lowercase();
    if query.is_empty() {
        return people.iter().collect();
    }
    let mut by_name = Vec::new();
    let mut by_address = Vec::new();
    for person in people {
        if person.name.to_lowercase().starts_with(&query) {
            by_name.push(person);
        } else if person.address.to_lowercase().starts_with(&query) {
            by_address.push(person);
        }
    }
    by_name.extend(by_address);
    by_name
}

/// The person an `@` mention adds to Cc: `person`, unless they are already on To or Cc.
pub fn joins_cc<'a>(person: &'a Person, to: &[Person], cc: &[Person]) -> Option<&'a Person> {
    let listed = to
        .iter()
        .chain(cc)
        .any(|other| other.address.eq_ignore_ascii_case(&person.address));
    (!listed).then_some(person)
}

/// Whether `text` talks about an attachment.
///
/// English, Chinese, Japanese, French, Spanish, and German. Latin words match whole tokens;
/// 附件 and 添付 match as the words they are, since those scripts are not space-separated.
pub fn mentions_attachment(text: &str) -> bool {
    if text.contains("附件") || text.contains("添付") {
        return true;
    }
    let tokens = tokens(text);
    for window in tokens.windows(2) {
        if window[0] == "ci" && (window[1] == "joint" || window[1] == "jointe") {
            return true;
        }
    }
    tokens.into_iter().any(|token| {
        matches!(
            token.as_str(),
            "attach"
                | "attached"
                | "attachment"
                | "attaching"
                | "enclosed"
                | "enclosure"
                | "ci-joint"
                | "ci-jointe"
                | "adjunto"
                | "adjunta"
                | "adjuntos"
                | "adjuntas"
                | "anbei"
                | "anhang"
        )
    })
}

/// The document has an attachment object.
pub fn has_attachment(doc: &Doc) -> bool {
    doc.nodes
        .iter()
        .any(|node| matches!(node, Node::Object(Object::Attachment(_))))
}

/// The body mentions an attachment and the document has none.
pub fn missing_attachment(doc: &Doc) -> bool {
    mentions_attachment(&body_text(doc)) && !has_attachment(doc)
}

fn body_text(doc: &Doc) -> String {
    let mut out = String::new();
    for node in &doc.nodes {
        if let Node::Para { runs, .. } = node {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&runs_text(runs));
        }
    }
    out
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| ch.is_whitespace() || is_separator(ch))
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

fn is_separator(ch: char) -> bool {
    matches!(
        ch,
        ',' | '.' | '!' | '?' | ';' | ':' | '"' | '\'' | '(' | ')' | '…' | '“' | '”' | '‘' | '’'
    )
}
