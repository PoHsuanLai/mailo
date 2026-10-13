//! Menu items and choices: a row in a menu, a segment of a segmented control, a picker's option.
//!
//! Case: Title Case verb phrases with no article ("Mark as Read", "Move to…"). "…" only when more input follows.

// The sidebar's menu, under the foot's chevron.
pub const CLEAR_TODAY: &str = "Clear Today";
pub const HISTORY: &str = "History";
pub const NEW_SPACE: &str = "New Space";
pub const SETTINGS: &str = "Settings…";
pub const HIDE_SIDEBAR: &str = "Hide Sidebar";
pub const SHOW_SIDEBAR: &str = "Show Sidebar";

/// A message waiting for its time, as the row that takes it back.
pub fn cancel_sending(title: &str) -> String {
    format!("Cancel Sending {title}")
}

// The "+" after a Space's account tiles.
pub const ADD_ACCOUNT: &str = "Add Account…";

// The list toolbar's ⋯.
pub const GROUP: &str = "Group";
pub const ROW_PROPERTIES: &str = "Row Properties";
pub const SYNC_NOW: &str = "Sync Now";
// Group ▸, the ways the loaded page is banded.
pub const GROUP_NONE: &str = "None";
pub const GROUP_SENDER: &str = "Sender";
pub const GROUP_DATE: &str = "Date";
pub const GROUP_LABEL: &str = "Label";
pub const GROUP_UNREAD: &str = "Unread";
// Row Properties ▸, the parts a row shows.
pub const PART_SNIPPET: &str = "Snippet";
pub const PART_PROVIDER: &str = "Provider";
pub const PART_CHIPS: &str = "Chips";
pub const PART_TIME: &str = "Time";

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
pub const DELETE_DRAFT: &str = "Delete Draft";
