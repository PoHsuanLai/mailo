//! What changed since a state (RFC 8620 §5.2), and one page of a query (§5.5).

use super::field::{malformed, string, strings, unsigned};
use crate::ProtoError;
use serde_json::Value;

/// Whether a changes call reported everything, or stopped at `maxChanges`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum More {
    /// Ask again from `new_state`.
    Yes,
    No,
}

/// A `/changes` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub old_state: String,
    pub new_state: String,
    pub more: More,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

impl Changes {
    /// Parse the arguments of a `Foo/changes` answer.
    pub fn parse(args: &Value) -> Result<Changes, ProtoError> {
        Ok(Changes {
            old_state: string(args, "oldState")?.to_owned(),
            new_state: string(args, "newState")?.to_owned(),
            more: match args.get("hasMoreChanges").and_then(Value::as_bool) {
                Some(true) => More::Yes,
                _ => More::No,
            },
            created: strings(args, "created")?,
            updated: strings(args, "updated")?,
            destroyed: strings(args, "destroyed")?,
        })
    }

    /// Whether nothing changed at all.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.updated.is_empty() && self.destroyed.is_empty()
    }
}

/// One page of an `Email/query`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPage {
    pub ids: Vec<String>,
    /// Where this page starts in the whole result.
    pub position: u64,
    /// How many match in all, when the server calculated it.
    pub total: Option<u64>,
    pub query_state: String,
}

impl QueryPage {
    /// Parse the arguments of an `Email/query` answer.
    pub fn parse(args: &Value) -> Result<QueryPage, ProtoError> {
        Ok(QueryPage {
            ids: strings(args, "ids")?,
            position: unsigned(args, "position", 0)?,
            total: match args.get("total") {
                None | Some(Value::Null) => None,
                Some(_) => Some(unsigned(args, "total", 0)?),
            },
            query_state: string(args, "queryState")?.to_owned(),
        })
    }
}

/// The `state` of a `/get` answer: the state a later `/changes` asks about.
pub fn state_of(args: &Value) -> Result<String, ProtoError> {
    args.get("state")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| malformed("a /get answer has no state"))
}
