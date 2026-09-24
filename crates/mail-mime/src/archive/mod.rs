//! Mail at rest in files: mbox, Maildir and single `.eml` messages, read and written.
//!
//! Pure in the sense this crate means it: no filesystem walk and no clock. An mbox is read from
//! any [`std::io::BufRead`] and written to any [`std::io::Write`], because the caller owns the
//! file and a Takeout archive of several gigabytes must stream through rather than be loaded.
//! Maildir is a directory layout, so what lives here is its naming — the `:2,` flag suffix,
//! unique names, `.Folder.Sub` — and the caller does the walking.
//!
//! What a file says about where a message belonged and what had happened to it is reduced to
//! one [`Placement`], whichever format said it: a Maildir flag suffix, a Takeout
//! `X-Gmail-Labels` header, or an mbox `Status` header.

pub mod maildir;
pub mod mbox;
pub mod takeout;

use mail_domain::{MailboxRole, ReadState, Star, SystemFlag};

/// Where a message sat and what had been done to it, as the file recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub role: MailboxRole,
    /// Sorted and without repeats.
    pub flags: Vec<SystemFlag>,
    /// Folder or label names that are not one of the roles, e.g. `Receipts` or `Work/2019`.
    pub labels: Vec<String>,
}

impl Placement {
    /// In `role`, with these flags and labels, tidied: flags sorted and unique, labels unique
    /// in the order given, and no empty label.
    pub fn new(role: MailboxRole, mut flags: Vec<SystemFlag>, labels: Vec<String>) -> Self {
        flags.sort();
        flags.dedup();
        let mut kept: Vec<String> = Vec::with_capacity(labels.len());
        for label in labels {
            let label = label.trim().to_owned();
            if !label.is_empty() && !kept.contains(&label) {
                kept.push(label);
            }
        }
        Self {
            role,
            flags,
            labels: kept,
        }
    }

    pub fn read(&self) -> ReadState {
        if self.flags.contains(&SystemFlag::Seen) {
            ReadState::Read
        } else {
            ReadState::Unread
        }
    }

    pub fn star(&self) -> Star {
        if self.flags.contains(&SystemFlag::Flagged) {
            Star::Starred
        } else {
            Star::Unstarred
        }
    }
}

/// What a file holding mail turned out to be, from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    /// Begins with a `From ` envelope line.
    Mbox,
    /// Anything else: taken to be one message.
    Eml,
}

/// Which format `head` — the first bytes of a file — is.
///
/// Leading blank lines are skipped, because some exporters write one before the first envelope.
pub fn sniff(head: &[u8]) -> Sniffed {
    let start = head
        .iter()
        .position(|b| !matches!(b, b'\r' | b'\n'))
        .unwrap_or(head.len());
    if head[start..].starts_with(b"From ") {
        Sniffed::Mbox
    } else {
        Sniffed::Eml
    }
}

/// The role a folder of this name plays, where the name is one clients agree on.
///
/// Only the last segment is read, so `[Gmail]/Sent Mail` and `Sent` agree. Case and the
/// difference between a space, a hyphen and nothing are ignored: `Junk E-mail`, `junk-email`.
pub fn role_for_folder(name: &str) -> Option<MailboxRole> {
    let last = name.rsplit(['/', '.']).next().unwrap_or(name);
    let folded: String = last
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    Some(match folded.as_str() {
        "inbox" => MailboxRole::Inbox,
        "sent" | "sentitems" | "sentmessages" | "sentmail" => MailboxRole::Sent,
        "drafts" | "draft" => MailboxRole::Drafts,
        "trash" | "deleted" | "deleteditems" | "deletedmessages" | "bin" => MailboxRole::Trash,
        "junk" | "spam" | "junkemail" | "junkmail" | "bulkmail" => MailboxRole::Spam,
        "archive" | "archives" | "allmail" => MailboxRole::Archive,
        _ => return None,
    })
}

/// A folder name as a placement: its role where it has one, else archived under a label of
/// the same name. `None` is the top of a mailbox, which is the inbox.
pub fn folder_placement(folder: Option<&str>, flags: Vec<SystemFlag>) -> Placement {
    match folder {
        None => Placement::new(MailboxRole::Inbox, flags, Vec::new()),
        Some(name) => match role_for_folder(name) {
            Some(role) => Placement::new(role, flags, Vec::new()),
            None => Placement::new(MailboxRole::Archive, flags, vec![name.to_owned()]),
        },
    }
}

/// The value of the first header called `name` in `raw`'s header section, unfolded.
///
/// Stops at the blank line that ends the headers, so a quoted message in the body is never
/// read as this one's. The value is decoded lossily: these are routing and status headers, not
/// text a person wrote.
pub fn header(raw: &[u8], name: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in lines(raw) {
        let line = trim_eol(line);
        if line.is_empty() {
            break;
        }
        if line[0] == b' ' || line[0] == b'\t' {
            if let Some(value) = found.as_mut() {
                value.push(' ');
                value.push_str(String::from_utf8_lossy(line).trim());
            }
            continue;
        }
        if found.is_some() {
            break;
        }
        if let Some(colon) = line.iter().position(|b| *b == b':')
            && line[..colon].eq_ignore_ascii_case(name.as_bytes())
        {
            found = Some(
                String::from_utf8_lossy(&line[colon + 1..])
                    .trim()
                    .to_owned(),
            );
        }
    }
    found
}

/// Every line terminated by LF becomes CRLF; a CR already before the LF is kept, not doubled.
///
/// RFC 5322's form, and what an IMAP server wants in an `APPEND` literal.
pub fn crlf(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 32);
    let mut previous = 0u8;
    for &b in raw {
        if b == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        previous = b;
    }
    out
}

/// Every CRLF becomes LF: the Unix form mbox and Maildir files are written in.
pub fn lf(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' && raw.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(raw[i]);
        i += 1;
    }
    out
}

/// `raw` split after each LF, the last piece possibly unterminated.
pub(crate) fn lines(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    raw.split_inclusive(|b| *b == b'\n')
}

/// A line without its CRLF or LF.
pub(crate) fn trim_eol(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_opens_with_an_envelope_line_is_an_mbox() {
        assert_eq!(sniff(b"From a@b Sat Jan  1 00:00:00 2022\n"), Sniffed::Mbox);
        assert_eq!(
            sniff(b"\r\nFrom a@b Sat Jan  1 00:00:00 2022\r\n"),
            Sniffed::Mbox
        );
        assert_eq!(sniff(b"From: a@b\r\nSubject: x\r\n\r\nhi"), Sniffed::Eml);
        assert_eq!(sniff(b""), Sniffed::Eml);
    }

    #[test]
    fn folder_names_clients_agree_on_have_roles() {
        for (name, role) in [
            ("INBOX", MailboxRole::Inbox),
            ("Sent Items", MailboxRole::Sent),
            ("[Gmail]/Sent Mail", MailboxRole::Sent),
            ("Junk E-mail", MailboxRole::Spam),
            ("Deleted Messages", MailboxRole::Trash),
            ("Drafts", MailboxRole::Drafts),
            ("Archive", MailboxRole::Archive),
        ] {
            assert_eq!(role_for_folder(name), Some(role), "{name}");
        }
        assert_eq!(role_for_folder("Receipts"), None);
    }

    #[test]
    fn a_folder_without_a_role_becomes_a_label_in_the_archive() {
        let placed = folder_placement(Some("Work/2019"), vec![SystemFlag::Seen]);
        assert_eq!(placed.role, MailboxRole::Archive);
        assert_eq!(placed.labels, vec!["Work/2019".to_owned()]);
        assert_eq!(placed.read(), ReadState::Read);
        assert_eq!(folder_placement(None, Vec::new()).role, MailboxRole::Inbox);
    }

    #[test]
    fn a_header_is_read_unfolded_and_only_from_the_header_section() {
        let raw =
            b"Subject: one\r\nX-Gmail-Labels: Inbox,\r\n Receipts\r\n\r\nX-Gmail-Labels: body\r\n";
        assert_eq!(
            header(raw, "x-gmail-labels").as_deref(),
            Some("Inbox, Receipts")
        );
        assert_eq!(header(b"Subject: a\n\nStatus: RO\n", "Status"), None);
    }

    #[test]
    fn line_endings_convert_both_ways_without_doubling() {
        assert_eq!(crlf(b"a\nb\r\nc"), b"a\r\nb\r\nc");
        assert_eq!(lf(b"a\r\nb\nc\r"), b"a\nb\nc\r");
        assert_eq!(crlf(&lf(b"x\r\ny\r\n")), b"x\r\ny\r\n");
    }
}
