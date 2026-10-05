//! What is known about a piece of text: who wrote it, who may see it. The router joins the labels
//! of everything an agent has read, so mail's words must say they are somebody else's.
//!
//! A label also comes in: every argument of an invocation carries the router's label for it, and
//! what mailo shows back of an argument (a preview's recipients) carries that label on, joined
//! with the label of anything mailo added to it. The join is the router's: integrity falls to the
//! lower, confidentiality rises to the higher and the Spaces are unioned, sources and classes are
//! unioned. Sources and classes mailo has no name for are kept as the router wrote them, so a
//! label passes through mailo without losing what it said.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The Space that stands for no Space in particular. Beside a real Space it says nothing more,
/// and the router drops it.
const DESKTOP: &str = "desktop";

/// Whether the content may steer an agent. `Untrusted < Trusted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    /// Written by someone else.
    Untrusted,
    /// Typed or chosen by the person, or authored by an app as metadata.
    Trusted,
}

/// Who may see it. `Public < Private(spaces) < Secret`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Confidentiality {
    /// Anywhere.
    Public,
    /// Inside these Spaces.
    Private(BTreeSet<String>),
    /// Never past the shell's trusted path.
    Secret,
}

impl Confidentiality {
    /// The confidentiality of anything made from both.
    fn join(&self, other: &Confidentiality) -> Confidentiality {
        use Confidentiality::{Private, Public, Secret};
        match (self, other) {
            (Secret, _) | (_, Secret) => Secret,
            (Public, Public) => Public,
            (Public, Private(spaces)) | (Private(spaces), Public) => {
                Private(normal(spaces.clone()))
            }
            (Private(a), Private(b)) => Private(normal(a.union(b).cloned().collect())),
        }
    }
}

/// `spaces` without the desktop when a real Space is among them.
fn normal(mut spaces: BTreeSet<String>) -> BTreeSet<String> {
    if spaces.len() > 1 {
        spaces.remove(DESKTOP);
    }
    spaces
}

/// Where the text came from, as the router writes it (`{"kind": "mail"}`, `{"kind": "app", "v":
/// ..}`). Kept as written: the router has more sources than mailo names, and one mailo cannot name
/// still has to go back out on what is made from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Source(serde_json::Value);

impl Source {
    /// Mail written by others.
    pub fn mail() -> Source {
        Source(serde_json::json!({ "kind": "mail" }))
    }

    /// The person's address book.
    pub fn contacts() -> Source {
        Source(serde_json::json!({ "kind": "contacts" }))
    }

    /// Metadata `app` wrote.
    pub fn app(app: &str) -> Source {
        Source(serde_json::json!({ "kind": "app", "v": app }))
    }
}

/// The kind of data it carries (`mail`, `contacts`), as the router writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Class(String);

impl Class {
    /// Mail.
    pub fn mail() -> Class {
        Class("mail".to_owned())
    }

    /// Contacts.
    pub fn contacts() -> Class {
        Class("contacts".to_owned())
    }
}

/// A label, as the router writes and reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            confidentiality: Confidentiality::Private(BTreeSet::from([space.to_owned()])),
            classes: vec![Class::mail()],
            sources: vec![Source::mail()],
        }
    }

    /// What the person keeps in their address book, private to `space`: theirs, so trusted.
    pub fn contacts(space: &str) -> Label {
        Label {
            integrity: Integrity::Trusted,
            confidentiality: Confidentiality::Private(BTreeSet::from([space.to_owned()])),
            classes: vec![Class::contacts()],
            sources: vec![Source::contacts()],
        }
    }

    /// Words mailo wrote itself (an id, a count, a sentence about what it did) for `app`.
    pub fn own(app: &str) -> Label {
        Label {
            integrity: Integrity::Trusted,
            confidentiality: Confidentiality::Public,
            classes: Vec::new(),
            sources: vec![Source::app(app)],
        }
    }

    /// The label of anything made from both.
    pub fn join(&self, other: &Label) -> Label {
        fn union<T: Clone + PartialEq>(a: &[T], b: &[T]) -> Vec<T> {
            let mut all = a.to_vec();
            all.extend(b.iter().filter(|x| !a.contains(x)).cloned());
            all
        }
        Label {
            integrity: self.integrity.min(other.integrity),
            confidentiality: self.confidentiality.join(&other.confidentiality),
            classes: union(&self.classes, &other.classes),
            sources: union(&self.sources, &other.sources),
        }
    }

    /// Whether the content may steer an agent.
    pub fn integrity(&self) -> Integrity {
        self.integrity
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
