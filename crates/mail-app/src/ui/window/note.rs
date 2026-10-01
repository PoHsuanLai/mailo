//! What a conversation's own window says when the conversation has left where it was.
//!
//! The main window's list lets an archived row go; a window showing only the reader has no row
//! to lose, so it says what happened instead, whichever window did it. The reader under the
//! note is drawn from the store as it is now, never from what the window opened on.

use mail_domain::{MailboxRole, MailboxSet};

/// What to say, when anything: `opened` is where the conversation was when its window opened,
/// `now` where it is. Trash and Spam are named before an archive, since either also takes it out
/// of the inbox. Brought back (an undo), it says nothing again.
pub(super) fn left(opened: MailboxSet, now: MailboxSet) -> Option<&'static str> {
    let arrived = |role| now.contains(role) && !opened.contains(role);
    if arrived(MailboxRole::Trash) {
        Some("Moved to Trash")
    } else if arrived(MailboxRole::Spam) {
        Some("Marked as spam")
    } else if opened.contains(MailboxRole::Inbox) && !now.contains(MailboxRole::Inbox) {
        Some("Archived: no longer in the Inbox")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(roles: &[MailboxRole]) -> MailboxSet {
        roles
            .iter()
            .fold(MailboxSet::empty(), |set, role| set.with(*role))
    }

    #[test]
    fn it_says_where_the_conversation_went_and_nothing_while_it_is_still_there() {
        use MailboxRole::*;
        const CASES: &[(&[MailboxRole], &[MailboxRole], Option<&str>)] = &[
            (&[Inbox], &[Inbox], None),
            (
                &[Inbox],
                &[Archive],
                Some("Archived: no longer in the Inbox"),
            ),
            (&[Inbox], &[], Some("Archived: no longer in the Inbox")),
            (&[Inbox], &[Trash], Some("Moved to Trash")),
            (&[Inbox], &[Spam], Some("Marked as spam")),
            (
                &[Inbox, Sent],
                &[Sent],
                Some("Archived: no longer in the Inbox"),
            ),
            // Opened from Archive: nothing left the inbox, and filing it back is not news.
            (&[Archive], &[Archive], None),
            (&[Archive], &[Inbox, Archive], None),
            (&[Archive], &[Trash], Some("Moved to Trash")),
            // Already in the Trash when it opened, and still there.
            (&[Trash], &[Trash], None),
        ];
        for (opened, now, want) in CASES {
            assert_eq!(
                left(set(opened), set(now)),
                *want,
                "opened in {opened:?}, now in {now:?}"
            );
        }
    }
}
