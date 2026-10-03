//! What the router sends: an invocation, the thing it names, the arguments it carries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One thing an app owns: the app, the kind (`mail.thread`) and the app's own key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId {
    /// The owning app.
    pub app: String,
    /// The kind.
    pub kind: String,
    /// Which one.
    pub key: String,
}

/// What an action acts on.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Target {
    /// Nothing in particular.
    Nothing,
    /// These things.
    Entities(Vec<EntityId>),
    /// A text field, by the app's token.
    Text(String),
    /// Files.
    Files(Vec<String>),
}

/// An argument's value. Only the forms an action of mailo takes are read; the rest are
/// [`Value::Other`], which every action refuses.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Tagged")]
pub enum Value {
    /// Words.
    Text(String),
    /// An instant, in seconds since the Unix epoch.
    DateTime(i64),
    /// One thing.
    Entity(EntityId),
    /// Several things.
    Entities(Vec<EntityId>),
    /// Any other form.
    Other,
}

/// A value as the router writes it, `{"kind": .., "v": ..}`, before its kind is known. (An
/// adjacently tagged enum cannot have a catch-all that skips the content of a kind it has no
/// variant for, so the kind is read first.)
#[derive(Deserialize)]
struct Tagged {
    kind: String,
    #[serde(default)]
    v: serde_json::Value,
}

impl TryFrom<Tagged> for Value {
    type Error = serde_json::Error;

    fn try_from(Tagged { kind, v }: Tagged) -> Result<Self, Self::Error> {
        Ok(match kind.as_str() {
            "text" => Value::Text(serde_json::from_value(v)?),
            "date_time" => Value::DateTime(serde_json::from_value(v)?),
            "entity" => Value::Entity(serde_json::from_value(v)?),
            "entities" => Value::Entities(serde_json::from_value(v)?),
            _ => Value::Other,
        })
    }
}

// It may be the person's mail or a recipient: Debug shows the shape and never the text.
impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Text(t) => write!(f, "Text(<{} bytes>)", t.len()),
            Value::DateTime(at) => write!(f, "DateTime({at})"),
            Value::Entity(_) => f.write_str("Entity(..)"),
            Value::Entities(all) => write!(f, "Entities(<{}>)", all.len()),
            Value::Other => f.write_str("Other"),
        }
    }
}

/// An argument and where it came from. Mailo does not read the label: the router has gated the
/// call on it already.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Arg {
    /// The value.
    pub value: Value,
}

/// What `Perform` and `DryRun` are given.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Invocation {
    /// The router's number for the call.
    pub call: u64,
    /// The action's name, `mail.thread.archive`.
    pub action: String,
    /// What it acts on.
    pub target: Target,
    /// What it takes, by parameter name.
    pub args: BTreeMap<String, Arg>,
    /// The Space it acts in: what labels on the answer are private to.
    pub space: String,
}

impl Invocation {
    /// The text argument `name`, when it was given as text.
    pub fn text(&self, name: &str) -> Option<&str> {
        match &self.args.get(name)?.value {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The instant argument `name`, when it was given as one.
    pub fn instant(&self, name: &str) -> Option<i64> {
        match self.args.get(name)?.value {
            Value::DateTime(at) => Some(at),
            _ => None,
        }
    }
}

/// An action of some app.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ActionRef {
    /// The app.
    pub app: String,
    /// The action.
    pub name: String,
}

/// What `Suggest` is asked: options for one parameter of one action.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SuggestAsk {
    /// The action.
    pub action: ActionRef,
    /// The parameter.
    pub param: String,
    /// What the person typed so far.
    pub typed: String,
}
