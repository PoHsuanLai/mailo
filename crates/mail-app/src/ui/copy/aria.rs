//! Accessible names: what a screen reader says for a control, including an icon-only button's `label:`.
//!
//! Case: Title Case for a control's name ("Sync Now", "More"); a fuller phrase may carry a value ("Settings for {name}").

// The sidebar.
pub const SIDEBAR: &str = "Sidebar";
/// The chevron at the foot's right.
pub const SIDEBAR_MENU: &str = "Sidebar Menu";
/// The foot's sync mark.
pub const SYNC_NOW: &str = "Sync Now";
/// The sync mark while an account in view waits for its sign-in: a press asks about it again.
pub const SIGN_IN_AGAIN: &str = "Sign In Again";
pub const FAVOURITES: &str = "Favourites";
pub const PLACES: &str = "Places";
pub const FOLDERS: &str = "Folders";

/// A row's ⋯: "Actions for Projects".
pub fn actions_for(name: &str) -> String {
    format!("Actions for {name}")
}

// The list's toolbar.
pub const SHOW_SIDEBAR: &str = "Show Sidebar";
pub const SAVE_AS_VIEW: &str = "Save as View";
pub const EDIT_VIEW: &str = "Edit View";
pub const NEW_MESSAGE: &str = "New Message";
pub const MORE: &str = "More";
/// The menu the ⋯ opens.
pub const LIST_OPTIONS: &str = "List options";
/// The menu a right click on the list's title opens: the places one step away.
pub const GO_TO: &str = "Go To";

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
