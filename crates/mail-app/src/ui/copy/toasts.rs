//! Toasts and notices: what the window says once, after something happened.
//!
//! Case: One short sentence, sentence case ("Moved to Archive.").

// A folder change, done.
pub fn folder_made(name: &str) -> String {
    format!("Folder “{name}” made")
}

pub fn folder_renamed(from: &str, to: &str) -> String {
    format!("“{from}” renamed to “{to}”")
}

pub fn folder_deleted(name: &str) -> String {
    format!("Folder “{name}” deleted")
}

pub fn folder_deleted_with_mail(name: &str) -> String {
    format!("Folder “{name}” deleted with its mail")
}

pub fn following(name: &str) -> String {
    format!("Following “{name}”")
}

pub fn stopped_following(name: &str) -> String {
    format!("Stopped following “{name}”")
}

/// Cancel Sending, done: the message is a draft again.
pub const SENDING_CANCELLED: &str = "Sending cancelled";

/// A file written into the downloads directory.
pub fn saved_to(path: &std::path::Path) -> String {
    format!("Saved to {}", path.display())
}

// An account moved to Microsoft Graph, or not.
pub fn graph_signed_in(name: &str) -> String {
    format!("{name} now reads and sends through Microsoft Graph.")
}

pub fn graph_waits(name: &str) -> String {
    format!("{name} is set up for Microsoft Graph and waits for you to sign in.")
}

pub fn graph_refused(name: &str, why: &str) -> String {
    format!("{name} couldn’t move to Microsoft Graph. {why}")
}
