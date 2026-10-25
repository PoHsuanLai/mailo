//! Tooltips: the name of a control, and its key, shown on hover.
//!
//! Case: A noun or verb phrase in Title Case, no full stop ("Sync", "New Message"). A tip never repeats a visible label.

/// The sidebar's sync mark, with nothing to say.
pub const SYNC: &str = "Sync";

/// The sync mark, with what its line says in a tip's fewer words: "Sync · 3 min ago".
pub fn sync_saying(said: &str) -> String {
    format!("{SYNC} · {said}")
}

/// The sync mark while an account in view waits for its sign-in: a press asks about it again.
pub const SIGN_IN_AGAIN: &str = "Sign In Again";

/// The "+" after the account tiles.
pub const ADD_ACCOUNT: &str = "Add Account";

pub const MORE: &str = "More";
pub const SHOW_SIDEBAR: &str = "Show Sidebar";
pub const SAVE_AS_VIEW: &str = "Save as View";
pub const EDIT_VIEW: &str = "Edit View";
pub const NEW_MESSAGE: &str = "New Message";
/// The list toolbar's button that shows the quick filter's row: what it filters by.
pub const FILTER: &str = "Filter by Unread, Starred or Attachments";

/// The selection bar's last button: unpick every row.
pub const CLEAR_SELECTION: &str = "Clear Selection";

// The glyph beside a protected message's sender, which opens what is known about it.
/// Encrypted, and opened for reading.
pub const ENCRYPTED: &str = "Encrypted";
/// Signed, and the signature checks.
pub const SIGNED: &str = "Signed";
/// Signed or encrypted, and nothing about it could be confirmed.
pub const NOT_VERIFIED: &str = "Not Verified";
