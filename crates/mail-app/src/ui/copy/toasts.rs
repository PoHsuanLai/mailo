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

/// A window that would not open: `what` is the window, as a person names it. Why goes to the
/// log; the toast says only what did not happen.
pub fn would_not_open(what: &str) -> String {
    format!("Couldn\u{2019}t open {what}.")
}
/// The window `would_not_open` names for a conversation's own window.
pub const A_NEW_WINDOW: &str = "a new window";

/// An account that came while the window was open, added in the Add Account window or by
/// `mailo account add`.
pub fn account_added(address: &str) -> String {
    format!("Added {address}")
}

/// Delete Draft, done: the toast that offers it back.
pub const DRAFT_DELETED: &str = "Draft deleted";

/// Cancel Sending, done: the message is a draft again.
pub const SENDING_CANCELLED: &str = "Sending cancelled";

/// A file written into the downloads directory.
pub fn saved_to(path: &std::path::Path) -> String {
    format!("Saved to {}", path.display())
}

// An account moved to Microsoft Graph, or not.
pub fn graph_signed_in(name: &str) -> String {
    format!("{name} now uses Microsoft Graph.")
}

pub fn graph_waits(name: &str) -> String {
    format!("{name} will use Microsoft Graph once you sign in.")
}

pub fn graph_refused(name: &str, why: &str) -> String {
    format!("Couldn’t switch {name} to Microsoft Graph. {why}")
}

// A program or script asked to open: shown in its folder instead.
pub fn shown_not_run(name: &str) -> String {
    format!("“{name}” is a program, so it’s shown in its folder rather than opened")
}

// Settings › Vacation Reply, kept.
pub const VACATION_OFF: &str = "Vacation reply off.";
/// The reply kept: its subject, and when it runs ([`super::status::vacation::runs`]).
pub fn vacation_saved(subject: &str, runs: &str) -> String {
    format!("Saved: “{subject}”, {runs}.")
}

// A calendar invitation and the calendar.
pub fn added_to_calendar(calendar: &str) -> String {
    format!("Added to {calendar}")
}

pub fn handed_to_calendar_app() -> String {
    "Opened in your calendar app".to_owned()
}

pub fn removed_from_calendar(calendar: &str) -> String {
    format!("Removed from {calendar}")
}
pub const NOT_IN_CALENDAR: &str = "It wasn’t in your calendar.";
pub const PROPOSAL_SENT: &str = "Proposal queued for the organiser.";

/// An answer queued, and the calendar kept in step.
pub fn answered_and_calendar(said: &str, calendar: &str) -> String {
    format!("{said}. {calendar}")
}

pub fn updated_in_calendar(calendar: &str) -> String {
    format!("Updated in {calendar}.")
}

pub fn taken_out_of_calendar(calendar: &str) -> String {
    format!("Removed from {calendar}.")
}
