//! Caps for the block walk, and the reason a body was cut.
//!
//! Values, not silent truncation: the body is still shown, and [`Reached`] says
//! which cap fired so the reader can tell the user the message was shortened.
//! [`Limits::version`] is its own number, not the sanitizer's — a mapping change
//! alters the blocks without altering the markup, and one shared version would
//! be bumped by whoever next edited the other.

/// Which cap, if any, stopped the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reached {
    /// The whole body fit.
    Nothing,
    /// Nesting passed [`Limits::MAX_DEPTH`]. Deeper tags were read as transparent.
    Depth,
    /// More than [`Limits::MAX_NODES`] elements.
    Nodes,
    /// More than [`Limits::MAX_BLOCKS`] blocks, or a table or list hit its cap.
    Blocks,
    /// More than [`Limits::MAX_TEXT`] characters, or one run passed [`Limits::MAX_RUN`].
    Text,
}

/// The caps, and the cache key that names this revision of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits;

impl Limits {
    /// Bumped when a cap or a tag mapping changes.
    ///
    /// Independent of [`crate::SanitizePolicy::version`]. The render cache must
    /// miss when the blocks would differ even if the sanitized markup would not.
    pub const fn version() -> u32 {
        1
    }

    /// Start tags past this depth are transparent and their end tags pop nothing.
    ///
    /// The walk itself is iterative. The cap is for the renderer, and for `Drop`
    /// of a nested [`Vec`](Vec), both of which recurse.
    pub const MAX_DEPTH: usize = 64;
    /// Elements, counting start tags.
    pub const MAX_NODES: u32 = 20_000;
    /// Blocks emitted into the document.
    pub const MAX_BLOCKS: usize = 5_000;
    /// Characters of text kept, across the whole body.
    pub const MAX_TEXT: usize = 2 * 1024 * 1024;
    /// Characters kept from a single text run.
    pub const MAX_RUN: usize = 64 * 1024;
    /// Rows kept in one table.
    pub const MAX_ROWS: usize = 1_000;
    /// Cells kept in one row.
    pub const MAX_COLS: usize = 64;
    /// Items kept in one list.
    pub const MAX_ITEMS: usize = 2_000;

    /// Heaviness at or above this is [`crate::block::Shape::Layout`].
    ///
    /// Each positive signal is capped below this, so two independent signals
    /// have to agree before a body is called heavy.
    pub const HEAVY: i32 = 100;
}
