//! Where a Space was left.

use mail_domain::ThreadId;
use porter_core::AccountId;
use serde::{Deserialize, Serialize};

/// The place, open thread and account tile a Space showed when you left it.
///
/// The place is stored by name, not position: a label added while you were elsewhere moves
/// every label after it, and "the third place" would then be a different place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Recall {
    /// The place's name, as the sidebar shows it. Empty is the first place.
    pub place: String,
    /// The thread open in the reader, if any.
    pub open: Option<ThreadId>,
    /// The account tile that was pressed. `None` is every account in the Space.
    pub account: Option<AccountId>,
}
