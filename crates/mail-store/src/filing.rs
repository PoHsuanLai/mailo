//! Where a message is filed when the server holds it in more than one mailbox.
//!
//! A message has one [`MailboxRole`], chosen when it first arrives from the folder it arrived
//! from. While only the inbox and Sent were fetched that was the whole story. Once every folder
//! is, a message can be held from two at once — a server copy in `INBOX` and in
//! `Projects/2026` is one message with two addresses — and which folder a first sync happens to
//! reach first would decide for ever whether it is listed in the inbox. These two rules keep the
//! role following the server instead. Both stores call them, so they cannot drift apart.
//!
//! Neither overrides the user: an ingest writes server truth and then puts every pending local
//! change back on top, so a message archived here and not yet moved there stays archived.

use mail_domain::{FolderRoles, MailboxRole};

/// A message we already hold has arrived again from a mailbox filed as `arrived_as`.
///
/// Only one move is made: from `Archive` into the inbox. An arrival from `INBOX` is the server
/// saying the message is in the inbox now, and `Archive` is what a copy found first in a user
/// folder was filed as. Arriving anywhere else says nothing about leaving the inbox — a copy in
/// a folder does not take it out — so nothing else changes here.
pub(crate) fn after_arrival(held: MailboxRole, arrived_as: MailboxRole) -> Option<MailboxRole> {
    (held == MailboxRole::Archive && arrived_as == MailboxRole::Inbox).then_some(MailboxRole::Inbox)
}

/// A message's address in `left` has gone from the server, and it is still held at
/// `remaining` (paths, in any order).
///
/// When that was its last address in `INBOX` and it is still filed there, it has been moved out
/// by another client, and it is filed where it went: the role of the first remaining folder,
/// by [`FolderRoles::filed_as`]. Without this, a message moved from the inbox to a folder
/// elsewhere stayed in the inbox here for as long as the folder copy existed.
///
/// With nothing remaining the message is deleted instead, as before, and this is not asked.
pub(crate) fn after_leaving(
    held: MailboxRole,
    left: &str,
    remaining: &[String],
    roles: &FolderRoles,
) -> Option<MailboxRole> {
    let inbox = |path: &str| path.eq_ignore_ascii_case("INBOX");
    if held != MailboxRole::Inbox || !inbox(left) || remaining.iter().any(|p| inbox(p)) {
        return None;
    }
    let mut paths: Vec<&String> = remaining.iter().collect();
    paths.sort();
    paths.first().map(|path| roles.filed_as(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles() -> FolderRoles {
        FolderRoles(vec![
            ("Trash".to_owned(), MailboxRole::Trash),
            ("Sent".to_owned(), MailboxRole::Sent),
        ])
    }

    fn paths(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn an_inbox_copy_brings_a_filed_message_into_the_inbox() {
        use MailboxRole::*;
        assert_eq!(after_arrival(Archive, Inbox), Some(Inbox));
        // A copy in a folder does not take a message out of the inbox.
        assert_eq!(after_arrival(Inbox, Archive), None);
        // Only what a user folder is filed as moves; a Sent or Trash copy stays put.
        assert_eq!(after_arrival(Sent, Inbox), None);
        assert_eq!(after_arrival(Trash, Inbox), None);
        assert_eq!(after_arrival(Archive, Archive), None);
    }

    #[test]
    fn leaving_the_inbox_files_it_where_it_went() {
        use MailboxRole::*;
        let r = roles();
        assert_eq!(
            after_leaving(Inbox, "INBOX", &paths(&["Projects/2026"]), &r),
            Some(Archive)
        );
        assert_eq!(
            after_leaving(Inbox, "INBOX", &paths(&["Trash"]), &r),
            Some(Trash)
        );
        // Still in the inbox under another spelling or address: nothing moved.
        assert_eq!(after_leaving(Inbox, "INBOX", &paths(&["Inbox"]), &r), None);
        // Leaving a folder is not leaving the inbox.
        assert_eq!(
            after_leaving(Inbox, "Projects/2026", &paths(&["INBOX"]), &r),
            None
        );
        // Filed elsewhere already, by the user or an earlier pass.
        assert_eq!(
            after_leaving(Archive, "INBOX", &paths(&["Projects/2026"]), &r),
            None
        );
        // Deterministic when several remain.
        assert_eq!(
            after_leaving(Inbox, "INBOX", &paths(&["Trash", "Projects/2026"]), &r),
            Some(Archive)
        );
    }
}
