//! Errors and remedies the window words itself.
//!
//! Case: Sentence case, a capital and a full stop; say what to do when there is something to do.

/// Who a sign-in is for, when the account has no name to say.
pub const THIS_ACCOUNT: &str = "this account";

// A folder change refused.
pub const POP3_NO_FOLDERS: &str = "POP3 has no folders.";
pub const LOCAL_HAS_LABELS: &str = "Local mail has labels, not folders.";
pub const ACCOUNT_GONE: &str = "That account is not set up here any more.";

pub fn special_folder(path: &str, special: &str) -> String {
    format!("“{path}” is the account’s {special} folder and stays as it is.")
}

pub fn folder_holds(path: &str, messages: &str) -> String {
    format!("“{path}” holds {messages} and was kept.")
}

pub fn folder_exists(name: &str) -> String {
    format!("There is already a folder called “{name}”.")
}

pub fn folder_unknown(name: &str) -> String {
    format!("“{name}” is not on the server’s list any more. Sync to refresh it.")
}

pub fn folder_has_children(name: &str) -> String {
    format!("“{name}” has folders inside it. Move or delete those first.")
}

pub fn bad_folder_name(name: &str, why: &str) -> String {
    format!("“{name}” can’t be a folder name: {why}.")
}

pub fn nothing_changed(why: &str) -> String {
    format!("Nothing was changed: {why}")
}

// A new folder's name refused before the server is asked.
pub const TYPE_A_NAME: &str = "Type a name first.";
pub const NAME_STARTS_WITH_FLAG: &str = "A folder name can’t start with “\\”.";
pub const NO_SUBFOLDERS: &str = "This server has no subfolders.";

pub fn name_has_delimiter(delimiter: char) -> String {
    format!("A folder name can’t contain “{delimiter}”.")
}
