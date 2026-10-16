//! Buttons: the words on a control a person presses.
//!
//! Case: Title Case ("Sync Now", "New Space"). "…" only when more input follows before anything happens.

/// The way out of a window with no account.
pub const ADD_ACCOUNT: &str = "Add Account…";
/// The Folders header's action: a folder at the top.
pub const NEW: &str = "New";
/// A folder's row asking before its mail goes with it.
pub const DELETE_FOLDER_AND_MAIL: &str = "Delete Folder and Mail";
/// The composer's one filled button.
pub const SEND: &str = "Send";
/// A sheet's way out that does nothing.
pub const CANCEL: &str = "Cancel";
/// A toast's button that puts back what was just deleted.
pub const UNDO: &str = "Undo";
/// Tidy Up's button: archive the conversations it lists.
pub fn archive_count(count: usize) -> String {
    format!("Archive {count}")
}
/// Tidy Up's ages, as its choice names them.
pub const ONE_WEEK: &str = "1 Week";
pub const TWO_WEEKS: &str = "2 Weeks";
pub const ONE_MONTH: &str = "1 Month";
pub const THREE_MONTHS: &str = "3 Months";
/// Keeps what a form or a field holds.
pub const SAVE: &str = "Save";
/// Takes one entry out of a list in Settings.
pub const REMOVE: &str = "Remove";
/// The Rules page's row that makes a rule.
pub const NEW_RULE: &str = "New Rule…";
/// Settings › Keyboard: every key as it ships, shown only while one differs.
pub const RESTORE_DEFAULTS: &str = "Restore Defaults";

/// The sign-in question's answer for an account Microsoft refused over IMAP.
pub const USE_GRAPH: &str = "Use Microsoft Graph";

// Peek: an attachment or a link's card, over the window.
/// Open the part in the app the system gives its kind.
pub const OPEN: &str = "Open";
/// A link's card: go there in the browser.
pub const OPEN_LINK: &str = "Open Link";
/// A link's card: put the address on the clipboard.
pub const COPY_LINK: &str = "Copy Link";

// The line over a message whose remote images are held back.
/// This conversation's images, now.
pub const LOAD_IMAGES: &str = "Load Images";

/// This sender's images, from now on.
pub fn always_load_from(email: &str) -> String {
    format!("Always Load from {email}")
}
