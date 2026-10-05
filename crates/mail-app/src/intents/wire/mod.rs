//! The JSON that crosses `org.quire.IntentProvider1`, written out here rather than shared.
//!
//! The desktop's router (docket) sends an invocation as JSON text and reads back JSON text, and
//! its Rust types are what define the forms. They live in a repository mailo does not depend on,
//! so this module mirrors the part mailo speaks: the serde forms are the router's (adjacently
//! tagged enums, `kind` and `v`; ids and units as bare strings and numbers), and
//! `tests/forms.rs` pins each one to the text the router writes or reads. A form the router adds
//! later is one this module has to be taught; one it already sends that mailo does not know
//! reads as [`Value::Other`], which no action accepts.
//!
//! Input types derive `Deserialize` only and output types `Serialize` only: mailo reads
//! invocations and writes answers, never the other way round.

mod answer;
mod call;
mod label;

pub use answer::{
    AppRefusal, Context, EntityRef, Follow, Here, Hit, Outcome, Output, Preview, Privacy,
    Selection, Snip, TextTarget, UndoFault, Undoable, Visible, answer_of,
};
pub use call::{ActionRef, EntityId, Invocation, SuggestAsk, Target, Value};
pub use label::{Integrity, Label, Labelled};

#[cfg(test)]
mod tests;
