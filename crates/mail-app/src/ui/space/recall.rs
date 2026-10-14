//! Where a Space was left.

use mail_domain::ThreadId;
use serde::{Deserialize, Serialize};

/// The place and open thread a Space showed when you left it.
///
/// The place is stored by its key (`view::place_key`), not position: a label added while you
/// were elsewhere moves every label after it, and "the third place" would then be a different
/// place. Nor by name alone: a label called Done is not the Done place.
///
/// A recall written while accounts had tiles also names the tile left pressed. It is passed
/// over: a Space is the only narrowing of accounts, and arriving shows all of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Recall {
    /// The place's key; a Space left before keys stored its name. Empty is the first place.
    pub place: String,
    /// The thread open in the reader, if any.
    pub open: Option<ThreadId>,
}
