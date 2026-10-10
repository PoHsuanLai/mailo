//! The composer's document, as edits.
//!
//! One column of paragraphs and atomic objects. A `beforeinput` event becomes one of five
//! operations — insert, delete, split, merge, set a kind or a mark — and each operation
//! returns the edit that undoes it. Nothing here touches a DOM.
//!
//! Besides the five, [`Op`] has the node-level edits the five cannot express on their own
//! and that an inverse needs: [`Op::InsertNodes`] (a paste of several paragraphs or an
//! object), [`Op::Replace`] (the inverse of a delete that crossed nodes), [`Op::SetLink`]
//! (a link is a mark with a value) and [`Op::Seq`] (several inverses undone as one).

mod convert;
mod doc;
mod error;
mod input;
mod keys;
mod markdown;
mod mention;
mod op;
mod paste;
mod session;
mod slash;
mod text;
mod undo;
mod write;

#[cfg(test)]
mod tests;

pub use convert::{nodes_from_blocks, nodes_from_plain};
pub use doc::{
    AttachmentRef, Check, Doc, ImageRef, Level, Mark, Marks, Node, Object, ParaKind, Pos, Presence,
    Range, Run, Table,
};
pub use error::OpError;
pub use input::{Edit, InputEvent, Record, interpret};
pub use keys::{Caret, Grip, Motion, backspace, enter};
pub use markdown::{Shortcut, shortcuts};
pub use mention::{
    Person, has_attachment, joins_cc, mentions_attachment, missing_attachment, resolve,
};
pub use op::{Op, apply, apply_all};
pub use session::Session;
pub use slash::{Action, Item, SNIPPET, catalog, filter, turn_into};
pub use text::{grapheme_len, node_len, para_len, runs_text};
pub use undo::{Burst, Log, PAUSE_MS};
pub use write::{to_flowed, to_html};
