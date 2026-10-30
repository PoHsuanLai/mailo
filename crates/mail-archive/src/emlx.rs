//! Apple Mail's own message files: `.emlx`, one message each.
//!
//! An `.emlx` is three things one after the other: a line holding the message's length in bytes,
//! the message itself (RFC 5322, LF line endings), and an XML property list Apple Mail keeps its
//! own facts in. Two of those facts matter here: `flags`, a bit field, and `date-received`, in
//! seconds since the epoch. A `.partial.emlx` is the same with its larger attachments stored
//! beside it rather than inside; what is inside is still a message, without those attachments
//! ([`is_partial`]).
//!
//! Apple Mail keeps the files in its library under `Name.mbox` packages and also writes them
//! into an export; the walking is the caller's, and everything decidable from one file is here.

use super::{Placement, folder_placement};
use chrono::{DateTime, Utc};
use mail_domain::{MailboxRole, SystemFlag};

// The bits of the `flags` property this reads. Apple Mail sets others: encrypted, recent,
// forwarded, a priority and an attachment count among them.
const READ: u64 = 1 << 0;
const DELETED: u64 = 1 << 1;
const ANSWERED: u64 = 1 << 2;
const FLAGGED: u64 = 1 << 4;
const DRAFT: u64 = 1 << 6;
const JUNK: u64 = 1 << 24;

/// Whether a file named `file` is a `.partial.emlx`: its larger attachments are stored beside it,
/// not in it.
pub fn is_partial(file: &str) -> bool {
    file.len() > ".partial.emlx".len()
        && file
            .get(file.len() - ".partial.emlx".len()..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(".partial.emlx"))
}

/// One `.emlx`, split.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Emlx<'a> {
    /// The message, as it was received.
    pub raw: &'a [u8],
    /// The `flags` property, where the file has one.
    pub flags: Option<u64>,
    /// The `date-received` property, where the file has one.
    pub received: Option<DateTime<Utc>>,
}

/// Split an `.emlx` into its message and its properties.
///
/// `None` when the first line is not a length, or the length runs past the end of the file:
/// then it is not an `.emlx`, or one cut short.
pub fn split(bytes: &[u8]) -> Option<Emlx<'_>> {
    let newline = bytes.iter().position(|b| *b == b'\n')?;
    let length: usize = std::str::from_utf8(&bytes[..newline])
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let start = newline + 1;
    let end = start.checked_add(length)?;
    let raw = bytes.get(start..end)?;
    let plist = String::from_utf8_lossy(&bytes[end..]);
    Some(Emlx {
        raw,
        flags: integer(&plist, "flags").and_then(|n| u64::try_from(n).ok()),
        received: integer(&plist, "date-received").and_then(|s| DateTime::from_timestamp(s, 0)),
    })
}

/// The `<integer>` that follows `<key>name</key>` in a property list.
///
/// A plain scan, not an XML parser: Apple Mail writes these lists one way, flat, and a key it
/// did not write reads as absent rather than failing the message.
fn integer(plist: &str, name: &str) -> Option<i64> {
    let key = format!("<key>{name}</key>");
    let after = &plist[plist.find(&key)? + key.len()..];
    let value = after.trim_start().strip_prefix("<integer>")?;
    value[..value.find("</integer>")?].trim().parse().ok()
}

/// Where a message with these `flags`, found in `folder` (`None` for no mailbox), belongs.
///
/// A message Apple Mail had marked deleted or as junk is placed in the trash or junk rather than
/// dropped, the way a trashed Maildir message is. With no flags at all it is taken as read,
/// because an archive is old mail.
pub fn placement(folder: Option<&str>, flags: Option<u64>) -> Placement {
    let Some(bits) = flags else {
        return folder_placement(folder, vec![SystemFlag::Seen]);
    };
    let mut system = Vec::new();
    for (bit, flag) in [
        (READ, SystemFlag::Seen),
        (ANSWERED, SystemFlag::Answered),
        (FLAGGED, SystemFlag::Flagged),
        (DRAFT, SystemFlag::Draft),
    ] {
        if bits & bit != 0 {
            system.push(flag);
        }
    }
    let mut placed = folder_placement(folder, system);
    if bits & DELETED != 0 {
        placed.role = MailboxRole::Trash;
    } else if bits & JUNK != 0 {
        placed.role = MailboxRole::Spam;
    }
    placed
}

/// The mailbox an Apple Mail package directory holds: `Work.mbox` is `Work`. `None` for any
/// other directory, which is Apple Mail's own storage inside a package rather than a mailbox.
pub fn folder_of_package(dir: &str) -> Option<&str> {
    dir.strip_suffix(".mbox").filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{ReadState, Star};

    #[test]
    fn a_partial_file_is_known_by_its_name() {
        assert!(is_partial("11.partial.emlx"));
        assert!(is_partial("11.PARTIAL.emlx"));
        assert!(!is_partial("11.emlx"));
        assert!(!is_partial(".partial.emlx"));
    }

    fn file(message: &[u8], plist: &str) -> Vec<u8> {
        let mut out = format!("{:<10}\n", message.len()).into_bytes();
        out.extend_from_slice(message);
        out.extend_from_slice(plist.as_bytes());
        out
    }

    const MESSAGE: &[u8] = b"From: Ada <ada@example.test>\nSubject: hi\n\nbody\n";

    const PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>date-received</key>\n\t<integer>1700000000</integer>\n\t<key>flags</key>\n\t<integer>8590195729</integer>\n</dict>\n</plist>\n";

    #[test]
    fn the_length_line_says_where_the_message_ends() {
        let bytes = file(MESSAGE, PLIST);
        let split = split(&bytes).expect("an emlx");
        assert_eq!(split.raw, MESSAGE);
        assert_eq!(split.flags, Some(8_590_195_729));
        assert_eq!(split.received, DateTime::from_timestamp(1_700_000_000, 0));
    }

    #[test]
    fn a_file_that_is_not_an_emlx_is_refused() {
        for bytes in [&b"From: a@b\n\nhi"[..], b"99\nshort", b"", b"12", b"-1\nx"] {
            assert_eq!(split(bytes), None, "{:?}", String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    fn a_file_without_properties_still_holds_its_message() {
        let bytes = file(MESSAGE, "");
        let split = split(&bytes).expect("an emlx");
        assert_eq!(split.raw, MESSAGE);
        assert_eq!(split.flags, None);
        assert_eq!(split.received, None);
    }

    #[test]
    fn the_flag_bits_become_flags_and_a_place() {
        for (bits, role, read, star) in [
            (
                Some(0),
                MailboxRole::Inbox,
                ReadState::Unread,
                Star::Unstarred,
            ),
            (
                Some(READ),
                MailboxRole::Inbox,
                ReadState::Read,
                Star::Unstarred,
            ),
            (
                Some(READ | FLAGGED),
                MailboxRole::Inbox,
                ReadState::Read,
                Star::Starred,
            ),
            (
                Some(READ | DELETED),
                MailboxRole::Trash,
                ReadState::Read,
                Star::Unstarred,
            ),
            (
                Some(JUNK),
                MailboxRole::Spam,
                ReadState::Unread,
                Star::Unstarred,
            ),
            (None, MailboxRole::Inbox, ReadState::Read, Star::Unstarred),
        ] {
            let placed = placement(None, bits);
            assert_eq!(
                (placed.role, placed.read(), placed.star()),
                (role, read, star),
                "{bits:?}"
            );
        }
        let answered = placement(Some("Work"), Some(ANSWERED | DRAFT));
        assert_eq!(
            answered.flags,
            vec![SystemFlag::Answered, SystemFlag::Draft]
        );
        assert_eq!(answered.labels, vec!["Work".to_owned()]);
    }

    #[test]
    fn a_package_names_its_mailbox() {
        assert_eq!(folder_of_package("Work.mbox"), Some("Work"));
        assert_eq!(
            folder_of_package("Sent Messages.mbox"),
            Some("Sent Messages")
        );
        assert_eq!(folder_of_package(".mbox"), None);
        assert_eq!(folder_of_package("Messages"), None);
    }
}
