//! Thunderbird's mail folders: an mbox per folder, named by the folder with no extension.
//!
//! A profile keeps each account's folders under `Mail/<account>/` (local and POP3) or
//! `ImapMail/<server>/` (what IMAP folders hold offline). `Inbox` is an mbox; beside it
//! `Inbox.msf` is Thunderbird's index of it, and a folder's subfolders sit in a directory named
//! `Inbox.sbd`. Deleting a message only marks it, in its `X-Mozilla-Status` header, until the
//! folder is compacted, so a marked message is still in the file and must be left behind.
//!
//! Walking the directories is the caller's; everything decidable from a name or a message is
//! here.

use super::header;

/// The `X-Mozilla-Status` bit that marks a message deleted but not yet compacted away.
const EXPUNGED: u32 = 0x0008;

/// The `X-Mozilla-Status` bit (`Partial` in Thunderbird's message flags) that marks a message of
/// an IMAP folder whose body Thunderbird fetched only a part of.
const PARTIAL: u32 = 0x0400;

/// The files beside the mboxes that are something else: indexes, filters, POP3 state, and
/// whatever else Thunderbird or an add-on keeps there.
const NOT_MAIL: [&str; 7] = ["msf", "dat", "json", "html", "sqlite", "js", "mab"];

/// Whether `raw` is a message Thunderbird had deleted and not yet compacted out of its file.
pub fn expunged(raw: &[u8]) -> bool {
    status(raw).is_some_and(|bits| bits & EXPUNGED != 0)
}

/// Whether `raw` is a message of which Thunderbird kept only a part: the file holds a cut-off
/// body, which is not the message.
pub fn truncated(raw: &[u8]) -> bool {
    status(raw).is_some_and(|bits| bits & PARTIAL != 0)
}

fn status(raw: &[u8]) -> Option<u32> {
    header(raw, "X-Mozilla-Status").and_then(|v| u32::from_str_radix(v.trim(), 16).ok())
}

/// The folder whose subfolders a `.sbd` directory holds: `Work.sbd` holds `Work`'s.
pub fn folder_of_children(dir: &str) -> Option<&str> {
    dir.strip_suffix(".sbd").filter(|name| !name.is_empty())
}

/// Whether a file in a Thunderbird account directory may be a folder's mbox, by its name: not
/// hidden, and not one of the files that sit beside the mboxes.
pub fn may_be_folder(file: &str) -> bool {
    if file.starts_with('.') {
        return false;
    }
    match file.rsplit_once('.') {
        Some((_, extension)) => !NOT_MAIL
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known)),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_marked_deleted_is_left_behind() {
        for (status, gone) in [
            ("0001", false),
            ("0009", true),
            ("0008", true),
            ("0000", false),
            ("zz", false),
        ] {
            let raw = format!("X-Mozilla-Status: {status}\r\nSubject: x\r\n\r\nbody");
            assert_eq!(expunged(raw.as_bytes()), gone, "{status}");
        }
        assert!(!expunged(b"Subject: x\r\n\r\nX-Mozilla-Status: 0008\r\n"));
    }

    #[test]
    fn a_message_with_a_body_cut_off_is_known() {
        for (status, cut) in [
            ("0001", false),
            ("0400", true),
            ("0401", true),
            ("0200", false),
        ] {
            let raw = format!("X-Mozilla-Status: {status}\r\nSubject: x\r\n\r\nbody");
            assert_eq!(truncated(raw.as_bytes()), cut, "{status}");
        }
    }

    #[test]
    fn subfolders_live_in_a_directory_named_for_their_parent() {
        assert_eq!(folder_of_children("Work.sbd"), Some("Work"));
        assert_eq!(folder_of_children(".sbd"), None);
        assert_eq!(folder_of_children("Work"), None);
    }

    #[test]
    fn only_files_without_a_known_extension_may_be_folders() {
        for (name, folder) in [
            ("Inbox", true),
            ("Sent", true),
            ("Receipts 2019", true),
            ("v1.2 notes", true),
            ("Inbox.msf", false),
            ("msgFilterRules.dat", false),
            ("popstate.dat", false),
            ("folderTree.json", false),
            (".DS_Store", false),
        ] {
            assert_eq!(may_be_folder(name), folder, "{name}");
        }
    }
}
