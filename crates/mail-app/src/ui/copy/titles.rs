//! Window, sheet, popover and page titles.
//!
//! Case: Title Case ("Edit Space", "Settings").

/// The list's title when no place is chosen.
pub const INBOX: &str = "Inbox";
/// The menu of accounts a new folder can go on.
pub const NEW_FOLDER: &str = "New Folder";
/// A compose window's title until its message has a subject.
pub const NEW_MESSAGE: &str = "New Message";
/// The sheet that archives the Inbox's old read conversations.
pub const TIDY_UP: &str = "Tidy Up";

// Settings: the window, its pages, and the pages they push.
pub const SETTINGS: &str = "Settings";
pub const GENERAL: &str = "General";
pub const ACCOUNTS: &str = "Accounts";
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

/// The question when Microsoft refused an account mail over IMAP and Graph would serve it.
pub fn use_graph(name: &str) -> String {
    format!("Use Microsoft Graph for {name}?")
}
