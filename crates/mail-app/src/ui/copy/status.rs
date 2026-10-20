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

/// A draft's row in ⌘K's Recent, under its name: "Draft · 14:05", "Draft · Tue".
pub fn draft_touched(when: &str) -> String {
    format!("Draft \u{b7} {when}")
}

/// The bar over a message held to leave at a time: "Leaves Tue 09:00".
pub fn leaves(when: &str) -> String {
    format!("Leaves {when}")
}

/// A draft's row named by who it goes to, when it has no subject: "To Ada Lovelace".
pub fn draft_to(who: &str) -> String {
    format!("To {who}")
}

/// "at 14:05", for an update an hour or more ago.
pub fn at_clock(clock: &str) -> String {
    format!("at {clock}")
}

/// The selection bar's count: "3 selected".
pub fn selected(count: usize) -> String {
    format!("{count} selected")
}

// Peek, while it draws an attachment or when it cannot.
/// A PDF's page, counted from 1.
pub fn page_of(page: u32, count: u32) -> String {
    format!("Page {page} of {count}")
}
pub const LOADING: &str = "Loading…";
pub const NOT_DOWNLOADED: &str = "Not downloaded yet";
pub const NO_PREVIEW: &str = "No preview for this file";
/// Why a part that should have drawn did not.
pub fn no_preview(why: &str) -> String {
    format!("No preview: {why}")
}
/// A forwarded message with no plain text to show.
pub const NO_TEXT: &str = "This message has no plain text";
/// Why anyview drew no card: what follows "No preview: ".
pub const TOOK_TOO_LONG: &str = "it took too long";
pub const TOO_LARGE_TO_PEEK: &str = "it’s too large";
pub fn no_attachment(index: usize) -> String {
    format!("there is no attachment {index}")
}

/// The trust popover's last line: what the receiving server checked about the sender, in the
/// checks' own words ("SPF pass · DKIM pass · DMARC pass — checked by mx.example.com").
pub fn sender_checks(said: &str) -> String {
    format!("Sender checks: {said}")
}

/// The quiet line over a message whose remote images are held back; its actions follow it.
pub const IMAGES_HIDDEN: &str = "Images hidden";

/// How many accounts a Space shows, beside its name in Settings › Spaces.
pub fn accounts_shown(count: usize) -> String {
    match count {
        0 => "No accounts".to_owned(),
        1 => "1 account".to_owned(),
        many => format!("{many} accounts"),
    }
}

/// Several of a Space's accounts are in trouble, as the All tile's mark says.
pub fn accounts_need_attention(count: usize) -> String {
    format!("{count} accounts need attention.")
}

/// Under the rules of an account whose server cannot run them: mailo runs them as it syncs.
pub const RULES_RUN_HERE: &str = "Runs only while mailo is open.";
