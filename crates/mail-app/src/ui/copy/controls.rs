//! Buttons: the words on a control a person presses.
//!
//! Case: Title Case ("Sync Now", "New Space"). "…" only when more input follows before anything happens.

/// The way out of a window with no account.
pub const ADD_ACCOUNT: &str = "Add Account…";
/// The Folders header's action: a folder at the top.
pub const NEW: &str = "New";
/// Settings › Spaces: the row that opens a new Space's page, before there is one.
pub const NEW_SPACE: &str = "New Space";
/// A new Space's page: the button that adds it and turns the windows to it.
pub const CREATE_SPACE: &str = "Create Space";
/// A Space's page in Settings: the button that asks before deleting it.
pub const DELETE_SPACE: &str = "Delete Space\u{2026}";
/// The question's confirming button when a Space is deleted.
pub const DELETE: &str = "Delete";
/// A folder's row asking before its mail goes with it.
pub const DELETE_FOLDER_AND_MAIL: &str = "Delete Folder and Mail";
/// The composer's one filled button.
pub const SEND: &str = "Send";
/// The composer's bar button that picks files to send.
pub const ATTACH: &str = "Attach";
/// A sheet's way out that does nothing.
pub const CANCEL: &str = "Cancel";
/// A dialog's one button, which closes it.
pub const OK: &str = "OK";
/// Opens the window on the defaults in place of settings files that cannot be read.
pub const USE_DEFAULTS: &str = "Use Defaults";
/// Ends mailo, leaving unreadable settings files as they are.
pub const QUIT: &str = "Quit";
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
pub const DOWNLOAD: &str = "Download";
/// Takes one entry out of a list in Settings.
pub const REMOVE: &str = "Remove";
/// An account's page in Settings: the row that opens a new alias's fields.
pub const ADD_ALIAS: &str = "Add Alias\u{2026}";
/// An alias's fields: the button that takes it off the account.
pub const REMOVE_ALIAS: &str = "Remove Alias";
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

// The line under a message that carries a one-time code.
/// Put the code on the clipboard.
pub const COPY_CODE: &str = "Copy Code";

/// Copy Code's accessible name, which says which code.
pub fn copy_code(code: &str) -> String {
    format!("Copy Code {code}")
}

// The line over a message whose remote images are held back.
/// This conversation's images, now.
pub const LOAD_IMAGES: &str = "Load Images";

/// This sender's images, from now on.
pub fn always_load_from(email: &str) -> String {
    format!("Always Load from {email}")
}

// The reader: a calendar invitation's one row.
/// The invitation row's disclosure: the event's facts, its people and its file.
pub const DETAILS: &str = "Details";
/// Under a rule's conditions: one more row.
pub const ADD_CONDITION: &str = "Add Condition";
/// A rule's conditions, from rows to the search language as text.
pub const EDIT_AS_TEXT: &str = "Edit as Text";
/// A rule's condition, from text back to rows.
pub const EDIT_AS_CONDITIONS: &str = "Edit as Conditions";
/// Makes mailo the application that opens email links (the launch question, Settings › General).
pub const MAKE_DEFAULT: &str = "Make Default";
/// Declines the launch question for good; Settings › General still offers it.
pub const NOT_NOW: &str = "Not Now";
/// Settings › Vacation Reply: keeps the reply and puts it on the server.
pub const KEEP_REPLY: &str = "Keep Reply";
/// The Saved-view editor's button that forgets the view.
pub const DELETE_VIEW: &str = "Delete View";
/// Asks again for what did not work: a download, a sign-in.
pub const TRY_AGAIN: &str = "Try Again";

/// Settings › Signatures: the row under an address's signatures that makes another.
pub const NEW_SIGNATURE: &str = "New Signature";
/// A signature's page in Settings: the button that deletes it.
pub const DELETE_SIGNATURE: &str = "Delete Signature";
