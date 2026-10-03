//! What is known about a piece of text: who wrote it, who may see it. The router joins the labels
//! of everything an agent has read, so mail's words must say they are somebody else's.

use serde::Serialize;

/// Whether the content may steer an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    /// Written by someone else.
    Untrusted,
    /// Authored by the app as metadata.
    Trusted,
}

/// Who may see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Confidentiality {
    /// Anywhere.
    Public,
    /// Inside these Spaces.
    Private(Vec<String>),
}

/// Where the text came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Source {
    /// Mail written by others.
    Mail,
    /// App-authored metadata.
    App(String),
}

/// The kind of data it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Mail.
    Mail,
}

/// A label, as the router writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    integrity: Integrity,
    confidentiality: Confidentiality,
    classes: Vec<Class>,
    sources: Vec<Source>,
}

impl Label {
    /// Words from a message, private to `space`: untrusted, whatever the account.
    pub fn mail(space: &str) -> Label {
        Label {
            integrity: Integrity::Untrusted,
            confidentiality: Confidentiality::Private(vec![space.to_owned()]),
            classes: vec![Class::Mail],
            sources: vec![Source::Mail],
        }
    }

    /// Words mailo wrote itself (an id, a count, a sentence about what it did) for `app`.
    pub fn own(app: &str) -> Label {
        Label {
            integrity: Integrity::Trusted,
            confidentiality: Confidentiality::Public,
            classes: Vec::new(),
            sources: vec![Source::App(app.to_owned())],
        }
    }
}

/// A value and its label.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct Labelled<T> {
    /// The value.
    pub value: T,
    /// Its label.
    pub label: Label,
}

// The person's mail may be in it: Debug shows the label and never the text.
impl<T> std::fmt::Debug for Labelled<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Labelled")
            .field("value", &"<redacted>")
            .field("label", &self.label)
            .finish()
    }
}
