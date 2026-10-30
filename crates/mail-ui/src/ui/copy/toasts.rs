//! Toasts and notices: what the window says once, after something happened.
//!
//! Case: One short sentence, sentence case ("Moved to Archive.").

use chrono::{DateTime, TimeZone, Utc};
use mail_client::when::{Clock, Stamp, stamp};
use mail_domain::{FollowUp, Membership, Mute, Op, Pin, ReadState, Snooze, Star};

/// A message's one-time code, put on the clipboard by Copy Code.
pub const CODE_COPIED: &str = "Code copied";

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

/// Save All, done: how many attachments were written, and the folder's name.
pub fn saved_all(count: usize, folder: &str) -> String {
    match count {
        1 => format!("Saved 1 attachment to {folder}"),
        n => format!("Saved {n} attachments to {folder}"),
    }
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

/// Delete Signature, done: its Undo puts it back.
pub fn signature_deleted(name: &str) -> String {
    format!("Signature “{name}” deleted")
}

/// Block Sender, done: what happens to the sender's mail from now on.
pub fn blocked(email: &str) -> String {
    format!("Mail from {email} now goes to Spam")
}

/// Block Sender, refused.
pub fn not_blocked(email: &str, why: &str) -> String {
    format!("{email} was not blocked: {why}")
}

/// A redirect queued: whom it goes to, as the envelope names them.
pub fn redirected(to: &[String]) -> String {
    format!("Redirected to {}", to.join(", "))
}

/// A redirect, a reply to the list, or Edit as New that did not start: `why` is the core's
/// sentence.
pub fn not_redirected(why: &str) -> String {
    format!("Not redirected: {why}.")
}

pub fn not_opened(why: &str) -> String {
    format!("Couldn\u{2019}t start that message: {why}.")
}

/// ⌘K's Redirect…, Reply to List or Edit as New with no conversation open behind it.
pub const OPEN_ONE_FIRST: &str = "Open a conversation first.";
/// ⌘K's Forward as Attachment with no conversation open behind it.
pub const OPEN_ONE_TO_FORWARD: &str = "Open a conversation to forward it as an attachment.";
/// ⌘K's Forward as Attachment, when the message could not be made.
pub fn not_forwarded_as_attachment(why: &str) -> String {
    format!("Not forwarded as an attachment: {why}.")
}
/// Add to Contacts made a card.
pub fn added_to_contacts(name: &str) -> String {
    format!("Added {name} to Contacts.")
}

/// What one operation did, in words: "Archived". A time in it is written as `clock` says.
pub fn performed<Tz: TimeZone>(op: &Op, zone: &Tz, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let when = |at: DateTime<Utc>| stamp(at, zone, Stamp::Weekday, clock);
    match op {
        Op::Archive => "Archived".to_owned(),
        Op::Trash => "Moved to Trash".to_owned(),
        Op::Restore => "Moved to Inbox".to_owned(),
        Op::Spam => "Marked as spam".to_owned(),
        Op::SetRead(ReadState::Read) => "Marked read".to_owned(),
        Op::SetRead(ReadState::Unread) => "Marked unread".to_owned(),
        Op::SetStar(Star::Starred) => "Starred".to_owned(),
        Op::SetStar(Star::Unstarred) => "Unstarred".to_owned(),
        Op::Flag(hue) => format!("Flagged {hue}"),
        Op::Label(_, Membership::In) => "Labelled".to_owned(),
        Op::Label(_, Membership::Out) => "Label removed".to_owned(),
        Op::SetSnooze(Snooze::Until(at)) => format!("Snoozed until {}", when(*at)),
        Op::SetSnooze(Snooze::Inactive) => "Back in the inbox".to_owned(),
        Op::SetPin(Pin::Rank(_)) => "Pinned".to_owned(),
        Op::SetPin(Pin::Unpinned) => "Unpinned".to_owned(),
        Op::SetMute(Mute::Muted) => "Muted".to_owned(),
        Op::SetMute(Mute::Unmuted) => "Unmuted".to_owned(),
        Op::SetFollowUp(FollowUp::Until { at, .. }) => {
            format!("Reminder set for {} if no reply", when(*at))
        }
        Op::SetFollowUp(FollowUp::Returned { .. }) => "No reply yet".to_owned(),
        Op::SetFollowUp(FollowUp::Inactive) => "Reminder cleared".to_owned(),
        Op::File(_) => "Moved to folder".to_owned(),
        Op::Copy(_) => "Copied to folder".to_owned(),
        Op::Destroy => "Deleted forever".to_owned(),
    }
}

/// What one gesture did to `count` conversations: "Archived · 3 conversations". One
/// conversation is said as [`performed`] says it.
pub fn performed_on<Tz: TimeZone>(op: &Op, count: usize, zone: &Tz, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let one = performed(op, zone, clock);
    if count == 1 {
        one
    } else {
        format!("{one} · {count} conversations")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gesture_on_several_conversations_counts_them() {
        assert_eq!(performed_on(&Op::Archive, 1, &Utc, Clock::H24), "Archived");
        assert_eq!(
            performed_on(&Op::Archive, 3, &Utc, Clock::H24),
            "Archived · 3 conversations"
        );
        assert_eq!(
            performed_on(&Op::Trash, 2, &Utc, Clock::H24),
            "Moved to Trash · 2 conversations"
        );
    }

    #[test]
    fn an_operation_is_said_as_what_it_did() {
        let at = Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap();
        type Make = fn(DateTime<Utc>) -> Op;
        const CASES: &[(Make, &str)] = &[
            (|_| Op::Archive, "Archived"),
            (|_| Op::Trash, "Moved to Trash"),
            (|_| Op::SetStar(Star::Starred), "Starred"),
            (
                |at| Op::SetSnooze(Snooze::Until(at)),
                "Snoozed until Thu 09:00",
            ),
        ];
        for (make, want) in CASES {
            assert_eq!(performed(&make(at), &Utc, Clock::H24), *want);
        }
    }
}
