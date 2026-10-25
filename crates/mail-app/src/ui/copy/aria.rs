//! Accessible names: what a screen reader says for a control, including an icon-only button's `label:`.
//!
//! Case: Title Case for a control's name ("Sync Now", "More"); a fuller phrase may carry a value ("Settings for {name}").

// The sidebar.
pub const SIDEBAR: &str = "Sidebar";
/// The chevron at the foot's right.
pub const SIDEBAR_MENU: &str = "Sidebar Menu";
/// The foot's sync mark.
pub const SYNC_NOW: &str = "Sync Now";
/// The row of account tiles at the sidebar's top.
pub const ACCOUNTS_IN_SPACE: &str = "Accounts in This Space";
/// Settings › Spaces' list under the Spaces, which holds New Space.
pub const ADD_A_SPACE: &str = "Add a Space";
/// The keys Settings list under the keys, which holds Make a Key and Import.
pub const ADD_A_KEY: &str = "Add a Key";
/// Settings › Rules' list under the rules, which holds New Rule.
pub const ADD_A_RULE: &str = "Add a Rule";
/// The "+" after the account tiles.
pub const ADD_ACCOUNT: &str = "Add Account";
/// The sync mark while an account in view waits for its sign-in: a press asks about it again.
pub const SIGN_IN_AGAIN: &str = "Sign In Again";
pub const FAVOURITES: &str = "Favourites";
pub const PLACES: &str = "Places";
pub const FOLDERS: &str = "Folders";

/// A row's menu: "Actions for Projects".
pub fn actions_for(name: &str) -> String {
    format!("Actions for {name}")
}

// The list's toolbar.
pub const SHOW_SIDEBAR: &str = "Show Sidebar";
pub const SAVE_AS_VIEW: &str = "Save as View";
pub const EDIT_VIEW: &str = "Edit View";
/// The Saved-view editor's button that forgets the view.
pub const DELETE_VIEW: &str = "Delete View";
pub const NEW_MESSAGE: &str = "New Message";
pub const MORE: &str = "More";
/// The menu the ⋯ opens.
pub const LIST_OPTIONS: &str = "List options";
/// The button that shows or hides the quick filter's row.
pub const FILTER: &str = "Filter";
/// The quick filter's row of toggles under the toolbar.
pub const QUICK_FILTER: &str = "Quick Filter";

// The composer's foot.
/// Send ▾: not "Send", which is the button beside it.
pub const SEND_OPTIONS: &str = "Send Options";
/// The composer's ⋯ menu.
pub const MESSAGE_ACTIONS: &str = "Message Actions";

/// A selection bar's button: "Move to Inbox the 3 selected".
pub fn on_selected(action: &str, count: usize) -> String {
    format!("{action} the {count} selected")
}
/// The selection bar's Delete Forever, shown while Trash or Spam is.
pub const DELETE_FOREVER: &str = "Delete Forever";
pub fn delete_selected_forever(count: usize) -> String {
    format!("Delete the {count} selected forever")
}
/// The selection bar's last button: unpick every row.
pub const CLEAR_THE_SELECTION: &str = "Clear the selection";
/// The paperclip on a row whose conversation carries files.
pub const HAS_ATTACHMENTS: &str = "Has Attachments";

/// The contact editor's Save, for the person it names.
pub fn save_contact(address: &str) -> String {
    format!("Save the Name for {address}")
}

/// A trusted sender's Remove.
pub fn stop_loading_images(sender: &str) -> String {
    format!("Stop Loading Images from {sender}")
}

/// An account's row in Settings, which opens its page.
pub fn details_for(account: &str) -> String {
    format!("Details for {account}")
}

/// A rule page's Save.
pub const SAVE_RULE: &str = "Save Rule";

// Peek.
/// A stored attachment's tile in the reader's strip, which opens it in Peek.
pub fn preview(name: &str) -> String {
    format!("Preview {name}")
}
/// A PDF's page buttons in Peek.
pub const PREVIOUS_PAGE: &str = "Previous Page";
pub const NEXT_PAGE: &str = "Next Page";
/// Peek showing a link's card.
pub const LINK_DESTINATION: &str = "Link Destination";
/// A PDF's page in Peek, counted from 1: the picture's description.
pub fn page_picture(name: &str, page: u32) -> String {
    format!("{name}, page {page}")
}

// The reader: a calendar invitation's disclosure, named for what pressing it does.
pub const SHOW_DETAILS: &str = "Show Details";
pub const HIDE_DETAILS: &str = "Hide Details";

// The reader: an earlier message folded to one line, named for what pressing it does.
pub fn open_message(sender: &str) -> String {
    format!("Show Message from {sender}")
}
/// A rule's condition row's menu of what it looks at.
pub const CONDITION_FIELD: &str = "Condition field";
/// A rule's condition row's menu of how it tests.
pub const CONDITION_TEST: &str = "Condition test";
/// A rule's condition row's button that takes it away.
pub const REMOVE_CONDITION: &str = "Remove condition";
/// A rule's button that opens the menu of actions.
pub const ADD_ACTION: &str = "Add an action";
/// An account tile's right-click menu.
pub const ACCOUNT_MENU: &str = "Account";
/// A row of the list, which opens it: "Open Offsite plans".
pub fn open_row(subject: &str) -> String {
    format!("Open {subject}")
}

/// A failed attachment's Try Again, naming the file.
pub fn try_again_to_download(name: &str) -> String {
    format!("Try again to download {name}")
}
