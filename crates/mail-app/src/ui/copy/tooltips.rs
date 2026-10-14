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

pub const MORE: &str = "More";
pub const SHOW_SIDEBAR: &str = "Show Sidebar";
pub const SAVE_AS_VIEW: &str = "Save as View";
pub const EDIT_VIEW: &str = "Edit View";
pub const NEW_MESSAGE: &str = "New Message";
