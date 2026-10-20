//! Errors and remedies the window words itself.
//!
//! Case: Sentence case, a capital and a full stop; say what to do when there is something to do.

/// Who a sign-in is for, when the account has no name to say.
pub const THIS_ACCOUNT: &str = "this account";

// A folder change refused.
pub const POP3_NO_FOLDERS: &str = "POP3 has no folders.";
pub const LOCAL_HAS_LABELS: &str = "Local mail has labels, not folders.";
pub const ACCOUNT_GONE: &str = "That account isn’t here any more.";

pub fn special_folder(path: &str, special: &str) -> String {
    format!("“{path}” is the account’s {special} folder and can’t be changed.")
}

pub fn folder_holds(path: &str, messages: &str) -> String {
    format!("“{path}” holds {messages} and was kept.")
}

pub fn folder_exists(name: &str) -> String {
    format!("A folder called “{name}” already exists.")
}

pub fn folder_unknown(name: &str) -> String {
    format!("“{name}” is no longer on the server. Sync to refresh.")
}

pub fn folder_has_children(name: &str) -> String {
    format!("“{name}” has folders inside it. Move or delete those first.")
}

pub fn bad_folder_name(name: &str, why: &str) -> String {
    format!("“{name}” can’t be a folder name: {why}.")
}

pub fn nothing_changed(why: &str) -> String {
    format!("Nothing changed: {why}")
}

// A new folder's name refused before the server is asked.
pub const TYPE_A_NAME: &str = "Type a name first.";
pub const NAME_STARTS_WITH_FLAG: &str = "A folder name can’t start with “\\”.";
pub const NO_SUBFOLDERS: &str = "This server has no subfolders.";

pub fn name_has_delimiter(delimiter: char) -> String {
    format!("A folder name can’t contain “{delimiter}”.")
}

/// A file picked to import that could not be read.
pub fn cannot_read(name: &str) -> String {
    format!("Can’t read {name}.")
}

/// A choice in Settings with nowhere to be written.
pub const NO_CONFIG_DIRECTORY: &str = "There’s nowhere to save this.";

/// A rule or vacation reply kept here that its server did not take.
pub fn not_on_server(why: &str) -> String {
    format!("Kept on this computer, but the server refused it: {why}")
}

/// Choose Time… refusing a time that has gone: snoozing until then would bring it straight back.
pub fn time_passed(when: &str) -> String {
    format!("{when} has passed. Pick a later time.")
}

/// Under [`super::titles::use_graph`]: why it is asked.
pub const IMAP_REFUSED: &str = "Microsoft refused IMAP for this account, perhaps because its \
    organisation turned it off. Microsoft Graph still works.";

/// A link's card, when the link's text names another place than the one it goes to.
pub fn link_goes_elsewhere(goes_to: &str, claims: &str) -> String {
    format!("This link says {claims} but goes to {goes_to}.")
}

/// Under a message's head when the sender's domain says the server that sent it is not its own.
pub fn may_not_be_from(sender: &str) -> String {
    format!("This may not be from {sender}.")
}

/// A Space's name field, emptied.
pub const SPACE_NEEDS_A_NAME: &str = "A Space needs a name.";

/// Why a `keyboard.json` naming one action twice cannot be used.
pub fn key_given_twice(action: &str) -> String {
    format!("{action} is given a key twice.")
}
