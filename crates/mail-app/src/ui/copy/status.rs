//! Status lines: what the window says while something is so (fresh, syncing, offline).
//!
//! Case: Sentence case, as short as says it ("Updated 3 minutes ago").

/// "1 message", "132 messages".
pub fn messages(count: u64) -> String {
    if count == 1 {
        "1 message".to_owned()
    } else {
        format!("{count} messages")
    }
}

// How fresh the mail is, in the sync mark's tip's fewer words.
pub const JUST_NOW: &str = "just now";

/// "1 min ago", "3 min ago".
pub fn minutes_ago(minutes: i64) -> String {
    format!("{minutes} min ago")
}

/// "at 14:05", for an update an hour or more ago.
pub fn at_clock(clock: &str) -> String {
    format!("at {clock}")
}
