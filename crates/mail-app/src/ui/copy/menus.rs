//! Menu items and choices: a row in a menu, a segment of a segmented control, a picker's option.
//!
//! Case: Title Case verb phrases with no article ("Mark as Read", "Move to…"). "…" only when more input follows.

// The sidebar's menu, under the foot's chevron.
pub const SETTINGS: &str = "Settings…";
/// The sidebar's menu: archive the read conversations the Inbox has kept too long.
pub const TIDY_UP: &str = "Tidy Up…";
pub const HIDE_SIDEBAR: &str = "Hide Sidebar";
pub const SHOW_SIDEBAR: &str = "Show Sidebar";

// A scheduled message's menu: it goes back to Drafts, with undo, so nothing more is asked.
pub const CANCEL_SENDING: &str = "Cancel Sending";

// ⌘K's commands that go to a place.
pub const GO_TO_INBOX: &str = "Go to Inbox";
pub const GO_TO_KEPT: &str = "Go to Kept";
pub const GO_TO_SNOOZED: &str = "Go to Snoozed";
pub const GO_TO_DONE: &str = "Go to Done";
pub const GO_TO_TRASH: &str = "Go to Trash";

// The Space's menu › Accounts.
pub const ACCOUNTS: &str = "Accounts";
/// Add Account when it opens the window itself.
pub const ADD_ACCOUNT: &str = "Add Account…";
/// Add Account when it is a submenu of the providers that sign in in the browser.
pub const ADD_ACCOUNT_SUBMENU: &str = "Add Account";

// The list toolbar's ⋯.
pub const GROUP_BY_DATE: &str = "Group by Date";
pub const SHOW_SNIPPETS: &str = "Show Snippets";
pub const HIDE_SNIPPETS: &str = "Hide Snippets";
pub const SHOW_LABELS: &str = "Show Labels";
pub const HIDE_LABELS: &str = "Hide Labels";
pub const SYNC_NOW: &str = "Sync Now";

// A folder's ⋯.
pub const NEW_FOLDER_INSIDE: &str = "New Folder Inside";
pub const RENAME: &str = "Rename";
pub const FOLLOW: &str = "Follow";
pub const STOP_FOLLOWING: &str = "Stop Following";
/// Under Stop Following: what it does beyond this window.
pub const STOP_FOLLOWING_HELP: &str = "Other mail apps may stop showing it.";
pub const DELETE: &str = "Delete";

// The row at the end of Folders that shows the folders nobody follows, or hides them again.
pub const SHOW_ALL_FOLDERS: &str = "Show All Folders";
pub const SHOW_FOLLOWED_ONLY: &str = "Show Followed Only";

// The composer's Send ▾ and ⋯.
pub const SEND_LATER: &str = "Send Later";
pub const REMIND_IF_NO_REPLY: &str = "Remind Me If No Reply";
pub const SECURITY: &str = "Security";
pub const SHOW_PLAIN_TEXT: &str = "Show Plain Text";
pub const SAVE_AS_TEMPLATE: &str = "Save as Template\u{2026}";
pub const KEEP_FOR_LATER: &str = "Keep for Later";
/// In a compose window: the draft moves to the main window's reader column.
pub const OPEN_IN_MAIN_WINDOW: &str = "Open in Main Window";
pub const DELETE_DRAFT: &str = "Delete Draft";

// A conversation's actions, in its row's menu, the reader's ⋯ and the selection bar: each named
// by the side the conversation is offered (`mail_core::view::Say`), never the side it is on.
pub const ARCHIVE: &str = "Archive";
/// Out of Archive or Trash, back to the inbox.
pub const MOVE_TO_INBOX: &str = "Move to Inbox";
/// Out of Spam, back to the inbox.
pub const NOT_SPAM: &str = "Not Spam";
pub const MARK_AS_SPAM: &str = "Mark as Spam";
pub const MOVE_TO_TRASH: &str = "Move to Trash";
pub const DELETE_FOREVER: &str = "Delete Forever…";
pub const MARK_AS_READ: &str = "Mark as Read";
pub const MARK_AS_UNREAD: &str = "Mark as Unread";
pub const STAR: &str = "Star";
pub const UNSTAR: &str = "Unstar";
pub const PIN: &str = "Pin";
pub const UNPIN: &str = "Unpin";
pub const MUTE: &str = "Mute";
pub const UNMUTE: &str = "Unmute";
pub const SNOOZE: &str = "Snooze…";
pub const UNSNOOZE: &str = "Unsnooze";
pub const REMIND: &str = "Remind Me If No Reply…";
pub const STOP_REMINDING: &str = "Stop Reminding";
pub const LABEL: &str = "Label…";
pub const REPLY: &str = "Reply";
pub const REPLY_ALL: &str = "Reply All";
pub const FORWARD: &str = "Forward";
pub const MOVE_TO: &str = "Move to…";
pub const DOWNLOAD: &str = "Download";
pub const PRINT: &str = "Print…";
pub const VIEW: &str = "View";

/// The word for the side of an action a conversation is offered.
pub fn offered(say: mail_core::view::Say) -> &'static str {
    use mail_core::view::Say;
    match say {
        Say::Archive => ARCHIVE,
        Say::MoveToInbox => MOVE_TO_INBOX,
        Say::NotSpam => NOT_SPAM,
        Say::Spam => MARK_AS_SPAM,
        Say::Trash => MOVE_TO_TRASH,
        Say::DeleteForever => DELETE_FOREVER,
        Say::MarkRead => MARK_AS_READ,
        Say::MarkUnread => MARK_AS_UNREAD,
        Say::Star => STAR,
        Say::Unstar => UNSTAR,
        Say::Pin => PIN,
        Say::Unpin => UNPIN,
        Say::Mute => MUTE,
        Say::Unmute => UNMUTE,
        Say::Snooze => SNOOZE,
        Say::Unsnooze => UNSNOOZE,
        Say::Remind => REMIND,
        Say::StopReminding => STOP_REMINDING,
        Say::Label => LABEL,
        Say::Reply => REPLY,
        Say::ReplyAll => REPLY_ALL,
        Say::Forward => FORWARD,
    }
}

// ⌘K's commands for the contact book, which has no page of its own.
pub const IMPORT_CONTACTS: &str = "Import Contacts…";
pub const EXPORT_CONTACTS: &str = "Export Contacts…";

// A rule's right-click menu in Settings › Rules.
pub const RUN_ON_EXISTING_MAIL: &str = "Run on Existing Mail";
pub const MOVE_UP: &str = "Move Up";
pub const MOVE_DOWN: &str = "Move Down";
pub const DELETE_RULE: &str = "Delete Rule";

// The sender's name in the reader, right-clicked.
pub const EDIT_CONTACT: &str = "Edit Contact…";
