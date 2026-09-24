//! Maildir naming: what a file's name says about its message, and how to name a new one.
//!
//! A Maildir is three directories. A message is written into `tmp/` under a unique name and
//! renamed into `new/`; a client that has seen it moves it to `cur/` and appends an info suffix,
//! `:2,` and then one letter per flag in ASCII order — `D` draft, `F` flagged, `P` passed
//! (forwarded), `R` replied, `S` seen, `T` trashed. Maildir++ adds folders as sibling
//! directories named `.Folder.Sub`, the dot separating levels.
//!
//! Walking the directories is the caller's; everything decidable from a name is here.

use super::{Placement, folder_placement};
use mail_domain::{MailboxRole, SystemFlag};

/// A letter of a Maildir info suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Flag {
    Draft,
    Flagged,
    Passed,
    Replied,
    Seen,
    Trashed,
}

impl Flag {
    /// The letter, which is also the sort order the spec requires.
    pub fn letter(self) -> char {
        match self {
            Flag::Draft => 'D',
            Flag::Flagged => 'F',
            Flag::Passed => 'P',
            Flag::Replied => 'R',
            Flag::Seen => 'S',
            Flag::Trashed => 'T',
        }
    }

    fn from_letter(c: char) -> Option<Flag> {
        Some(match c {
            'D' => Flag::Draft,
            'F' => Flag::Flagged,
            'P' => Flag::Passed,
            'R' => Flag::Replied,
            'S' => Flag::Seen,
            'T' => Flag::Trashed,
            _ => return None,
        })
    }
}

/// Which of a Maildir's subdirectories a message was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sub {
    /// Delivered and not yet seen by any client. Carries no info suffix.
    New,
    /// Seen by a client, whatever its flags say about the user.
    Cur,
}

/// A message file's name, split: the unique part and the flags of its info suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub unique: String,
    /// Sorted, without repeats. Letters this module does not know are dropped.
    pub flags: Vec<Flag>,
}

/// Read a message file's name.
///
/// The info separator is `:`, or `;` or `!` where the filesystem forbids colons and a client
/// substituted one. Only `2,` info carries flags; `1,` is the spec's experimental form, and a
/// name with it, or with no info at all, is all unique part and has no flags.
pub fn parse_name(file: &str) -> Name {
    let split = file
        .rfind(':')
        .or_else(|| file.rfind(';'))
        .or_else(|| file.rfind('!'))
        .filter(|at| file[at + 1..].starts_with("2,"));
    let Some(at) = split else {
        return Name {
            unique: file.to_owned(),
            flags: Vec::new(),
        };
    };
    let mut flags: Vec<Flag> = file[at + 3..]
        .chars()
        .filter_map(Flag::from_letter)
        .collect();
    flags.sort();
    flags.dedup();
    Name {
        unique: file[..at].to_owned(),
        flags,
    }
}

/// A file name for `unique` with these flags: `unique:2,FS`.
pub fn name(unique: &str, flags: &[Flag]) -> String {
    let mut sorted = flags.to_vec();
    sorted.sort();
    sorted.dedup();
    let letters: String = sorted.into_iter().map(Flag::letter).collect();
    format!("{unique}:2,{letters}")
}

/// What makes a name unique: the spec's `time.MusecPpidQcount.host`.
///
/// Values rather than a clock and a pid, so this stays a function; the caller reads them. Two
/// deliveries in one microsecond from one process differ by `count`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unique<'a> {
    pub secs: i64,
    pub micros: u32,
    pub pid: u32,
    pub count: u64,
    pub host: &'a str,
}

/// A unique name, with `/` and `:` in the host name escaped as the spec says (`\057`, `\072`).
pub fn unique(parts: &Unique<'_>) -> String {
    let host = parts.host.replace('/', "\\057").replace(':', "\\072");
    let host = if host.is_empty() { "localhost" } else { &host };
    format!(
        "{}.M{}P{}Q{}.{host}",
        parts.secs, parts.micros, parts.pid, parts.count
    )
}

/// The folder a Maildir++ directory holds, as a `/`-separated path: `.Work.2019` is `Work/2019`.
///
/// `None` for anything that is not a Maildir++ folder: `cur`, `new`, `tmp`, `.` and `..`, and
/// names that are all dots.
pub fn folder_of_dir(dir: &str) -> Option<String> {
    let inner = dir.strip_prefix('.')?;
    let parts: Vec<&str> = inner.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// The Maildir++ directory for a `/`-separated folder path.
///
/// A dot inside a name would read back as a level, so it becomes `_`, as does anything a
/// directory name cannot hold.
pub fn dir_of_folder(path: &str) -> String {
    let parts: Vec<String> = path
        .split('/')
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.chars()
                .map(|c| match c {
                    '.' | '/' | '\\' | '\0' => '_',
                    c => c,
                })
                .collect()
        })
        .collect();
    format!(".{}", parts.join("."))
}

/// Where a message found in `folder` (`None` for the Maildir's top) under `sub` belongs.
///
/// Mail in `new/` is unread whatever its name says; nothing has seen it. A trashed message is
/// placed in the trash rather than flagged for deletion: this client never expunges.
pub fn placement(folder: Option<&str>, sub: Sub, flags: &[Flag]) -> Placement {
    let mut system = Vec::new();
    for flag in flags {
        match flag {
            Flag::Seen if sub == Sub::Cur => system.push(SystemFlag::Seen),
            Flag::Flagged => system.push(SystemFlag::Flagged),
            Flag::Replied => system.push(SystemFlag::Answered),
            Flag::Draft => system.push(SystemFlag::Draft),
            Flag::Seen | Flag::Passed | Flag::Trashed => {}
        }
    }
    let mut placed = folder_placement(folder, system);
    if flags.contains(&Flag::Trashed) {
        placed.role = MailboxRole::Trash;
    }
    placed
}

/// The Maildir flags for a message with these system flags.
pub fn flags_of(system: &[SystemFlag]) -> Vec<Flag> {
    let mut out: Vec<Flag> = system
        .iter()
        .map(|flag| match flag {
            SystemFlag::Seen => Flag::Seen,
            SystemFlag::Answered => Flag::Replied,
            SystemFlag::Flagged => Flag::Flagged,
            SystemFlag::Draft => Flag::Draft,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The Maildir++ folder a message in `role` is written to: `None` is the top, the inbox.
pub fn folder_for_role(role: MailboxRole) -> Option<&'static str> {
    match role {
        MailboxRole::Inbox => None,
        MailboxRole::Archive => Some("Archive"),
        MailboxRole::Sent => Some("Sent"),
        MailboxRole::Drafts => Some("Drafts"),
        MailboxRole::Trash => Some("Trash"),
        MailboxRole::Spam => Some("Junk"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{ReadState, Star};

    #[test]
    fn the_info_suffix_names_the_flags() {
        let parsed = parse_name("1700000000.M1P2Q3.host:2,SRF");
        assert_eq!(parsed.unique, "1700000000.M1P2Q3.host");
        assert_eq!(parsed.flags, vec![Flag::Flagged, Flag::Replied, Flag::Seen]);
        assert_eq!(parse_name("123.abc").flags, Vec::new());
        assert_eq!(parse_name("123.abc;2,S").flags, vec![Flag::Seen]);
        // `1,` is experimental info and says nothing about flags.
        assert_eq!(parse_name("123.abc:1,S").unique, "123.abc:1,S");
    }

    #[test]
    fn a_name_lists_its_flags_in_ascii_order_once() {
        assert_eq!(
            name("u", &[Flag::Seen, Flag::Draft, Flag::Seen, Flag::Flagged]),
            "u:2,DFS"
        );
        assert_eq!(name("u", &[]), "u:2,");
    }

    #[test]
    fn a_unique_name_escapes_what_the_spec_says() {
        let made = unique(&Unique {
            secs: 1_700_000_000,
            micros: 42,
            pid: 7,
            count: 3,
            host: "box/one:two",
        });
        assert_eq!(made, "1700000000.M42P7Q3.box\\057one\\072two");
        assert!(!made.contains('/') && !made.contains(':'));
    }

    #[test]
    fn maildir_plus_plus_folders_are_paths() {
        assert_eq!(folder_of_dir(".Work.2019").as_deref(), Some("Work/2019"));
        assert_eq!(folder_of_dir("cur"), None);
        assert_eq!(folder_of_dir(".."), None);
        assert_eq!(dir_of_folder("Work/2019"), ".Work.2019");
        assert_eq!(dir_of_folder("v1.2"), ".v1_2");
    }

    #[test]
    fn flags_become_state_and_new_mail_is_unread() {
        let cur = placement(Some("Receipts"), Sub::Cur, &[Flag::Seen, Flag::Flagged]);
        assert_eq!(cur.read(), ReadState::Read);
        assert_eq!(cur.star(), Star::Starred);
        assert_eq!(cur.role, MailboxRole::Archive);
        assert_eq!(cur.labels, vec!["Receipts"]);
        let new = placement(None, Sub::New, &[Flag::Seen]);
        assert_eq!(new.read(), ReadState::Unread);
        assert_eq!(new.role, MailboxRole::Inbox);
        let trashed = placement(None, Sub::Cur, &[Flag::Trashed, Flag::Replied]);
        assert_eq!(trashed.role, MailboxRole::Trash);
        assert_eq!(trashed.flags, vec![SystemFlag::Answered]);
    }

    #[test]
    fn system_flags_round_trip_through_maildir_letters() {
        let system = vec![
            SystemFlag::Seen,
            SystemFlag::Answered,
            SystemFlag::Flagged,
            SystemFlag::Draft,
        ];
        let letters = flags_of(&system);
        assert_eq!(name("u", &letters), "u:2,DFRS");
        let back = placement(None, Sub::Cur, &parse_name(&name("u", &letters)).flags);
        let mut sorted = system.clone();
        sorted.sort();
        assert_eq!(back.flags, sorted);
    }
}
