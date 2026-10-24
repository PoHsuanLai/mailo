//! Window, sheet, popover and page titles.
//!
//! Case: Title Case ("Edit Space", "Settings").

/// The "+" menu after the account tiles: the accounts the Space can show, and a new one.
pub const ADD_TO_SPACE: &str = "Add to This Space";
/// The list's title when no place is chosen.
pub const INBOX: &str = "Inbox";
/// The menu of accounts a new folder can go on.
pub const NEW_FOLDER: &str = "New Folder";
/// The sheet that archives the Inbox's old read conversations.
pub const TIDY_UP: &str = "Tidy Up";

// Settings: the window, its pages, and the pages they push.
pub const SETTINGS: &str = "Settings";
pub const GENERAL: &str = "General";
pub const ACCOUNTS: &str = "Accounts";
pub const SPACES: &str = "Spaces";
pub const RULES: &str = "Rules";
pub const KEYBOARD: &str = "Keyboard";
/// General's page of the senders whose images load.
pub const TRUSTED_SENDERS: &str = "Trusted Senders";
/// The page of the keys and certificates no account here owns, under Accounts.
pub const OTHER_KEYS: &str = "Other Keys";
/// A rule's page before it is kept.
pub const NEW_RULE: &str = "New Rule";
/// The vacation reply's page under Rules.
pub const VACATION_REPLY: &str = "Vacation Reply";
/// Settings' page of every address's signatures.
pub const SIGNATURES: &str = "Signatures";

/// The question when Microsoft refused an account mail over IMAP and Graph would serve it.
pub fn use_graph(name: &str) -> String {
    format!("Use Microsoft Graph for {name}?")
}

// The popover a protected message's glyph opens, titled as the glyph's tip names it.
pub const ENCRYPTED: &str = "Encrypted";
pub const SIGNED: &str = "Signed";
pub const NOT_VERIFIED: &str = "Not Verified";

// A failure in Settings, as its dialog's title; the reason is the dialog's message.
/// A setting, a sender or an account's switch that could not be written: nothing changed.
pub const NOT_SAVED: &str = "The Change Wasn’t Saved";
/// A key the Keyboard page cannot take for an action.
pub const KEY_NOT_USED: &str = "That Key Can’t Be Used";
/// "Check for mail when quit" that the system would not start at login, or stop starting.
pub const LOGIN_ITEM_NOT_CHANGED: &str = "Login Items Weren’t Changed";
/// "Check for mail when quit" that macOS keeps but waits for the person to allow.
pub const ALLOW_LOGIN_ITEM: &str = "Allow mailo in Login Items";
/// Remove Account that did not remove it.
pub const NOT_REMOVED: &str = "The Account Wasn’t Removed";
/// A rule or vacation reply kept here that its server did not take.
pub const NOT_ON_SERVER: &str = "Not Saved on the Server";
/// A rule run on existing mail that stopped.
pub fn did_not_run(name: &str) -> String {
    format!("“{name}” Didn’t Run")
}
/// A key or certificate job (import, save, delete, verify) that failed.
pub const KEYS_FAILED: &str = "That Didn’t Work";

/// Asked before the window opens over settings files that are there and cannot be read.
pub const SETTINGS_UNREADABLE: &str = "Some Settings Can’t Be Read";
/// Use Defaults could not set a damaged settings file aside; the reason is the message.
pub const NOT_SET_ASIDE: &str = "The Files Weren’t Set Aside";
