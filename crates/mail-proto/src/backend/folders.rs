//! Mailbox management over IMAP: the walks `ProtoOp::Folder` becomes, reading a listing, and
//! telling a refusal that means "already done" from one that means "no".
//!
//! Separate from `imap.rs` because none of it touches a message, and that file is long enough.

use crate::imap::ImapCommand;
use crate::machine::ProtoError;
use crate::mutf7;
use mail_domain::{
    Folder, FolderRoles, FolderWork, Holds, MailboxRole, NonEmpty, SpecialUse, Subscription,
};
use porter_core::AccountId;

/// The commands one piece of folder work is, in order, on one connection.
pub(crate) fn folder_commands(work: &FolderWork) -> Vec<ImapCommand> {
    match work {
        // Subscribed as well as created: a client that shows only what the user follows —
        // which is how most phones behave — would otherwise never show the folder at all.
        FolderWork::Create { path } => vec![
            ImapCommand::Create {
                mailbox: path.clone(),
            },
            ImapCommand::Subscribe {
                mailbox: path.clone(),
            },
        ],
        FolderWork::Rename { from, to } => vec![ImapCommand::Rename {
            from: from.clone(),
            to: to.clone(),
        }],
        FolderWork::Delete { path, non_empty } => {
            let delete = ImapCommand::Delete {
                mailbox: path.clone(),
            };
            match non_empty {
                // Asked again of the server, in the same walk as the delete. This client may
                // never have synced the folder, so its own count of what is in there proves
                // nothing, and a count taken on an earlier connection could be stale.
                NonEmpty::Refuse => vec![
                    ImapCommand::RequireEmpty {
                        mailbox: path.clone(),
                    },
                    delete,
                ],
                NonEmpty::Allow => vec![delete],
            }
        }
        FolderWork::Subscribe { path, subscription } => vec![match subscription {
            Subscription::Subscribed => ImapCommand::Subscribe {
                mailbox: path.clone(),
            },
            Subscription::Unsubscribed => ImapCommand::Unsubscribe {
                mailbox: path.clone(),
            },
        }],
    }
}

/// Every mailbox a `LIST` named, with whether an `LSUB` in the same walk named it too.
///
/// Parsed with `imap-proto` rather than by scanning text: a name may be quoted, an atom or a
/// literal, and may contain the delimiter, a quote or a space, none of which a split on `"`
/// survives. `LSUB` parses to the same shape and is told apart by its keyword. A name only
/// `LSUB` gives is a subscription to a mailbox that no longer exists, and is left out.
pub(crate) fn listing(untagged: &[crate::Untagged], account: AccountId) -> Vec<Folder> {
    let mut listed: Vec<Folder> = Vec::new();
    let mut followed: Vec<String> = Vec::new();
    for u in untagged {
        let Ok((
            _,
            imap_proto::Response::MailboxData(imap_proto::MailboxDatum::List {
                name_attributes,
                delimiter,
                name,
            }),
        )) = imap_proto::parser::parse_response(&u.raw)
        else {
            continue;
        };
        // `INBOX` in whatever case the server spells it is the inbox (RFC 3501 §5.1), and it is
        // `INBOX` here: Exchange lists `Inbox`, and a path kept as the server spelled it is a
        // second inbox to every comparison that does not know the rule.
        let name = match quoted_name(&u.raw) {
            true => unescape(&name),
            false => name.into_owned(),
        };
        let path = match mutf7::decode(&name) {
            inbox if inbox.eq_ignore_ascii_case("INBOX") => "INBOX".to_owned(),
            path => path,
        };
        // `* LSUB …` — the keyword is the second word, and a whole word.
        let lsub = u
            .text
            .split_whitespace()
            .nth(1)
            .is_some_and(|w| w.eq_ignore_ascii_case("LSUB"));
        if lsub {
            followed.push(path);
            continue;
        }
        // The inbox is the inbox by its name, whatever attributes ride along: a server that
        // lists `INBOX` with `\Flagged` (Twisted sends a mailbox's message flags there) has
        // not made its inbox a view of starred mail.
        let special = if path == "INBOX" {
            Some(SpecialUse::Inbox)
        } else {
            name_attributes.iter().find_map(special_use)
        };
        let holds = if name_attributes.iter().any(holds_nothing) {
            Holds::FoldersOnly
        } else {
            Holds::Mail
        };
        listed.push(Folder {
            account: account.clone(),
            path,
            delimiter: delimiter.and_then(|d| d.chars().next()),
            special,
            subscription: Subscription::Unsubscribed,
            holds,
        });
    }
    for folder in &mut listed {
        if followed.contains(&folder.path) {
            folder.subscription = Subscription::Subscribed;
        }
    }
    listed.sort_by(|a, b| a.path.cmp(&b.path));
    listed.dedup_by(|a, b| a.path == b.path);
    listed
}

/// Whether a `LIST` line's name, its last field, was a quoted string.
///
/// `imap-proto` hands a quoted name over as it stood between the quotes, escapes and all, and
/// an atom or a literal with nothing to undo, without saying which it was. A line that ends in
/// a quote and carries no literal ended in a quoted name.
fn quoted_name(raw: &[u8]) -> bool {
    let line = raw.trim_ascii_end();
    line.ends_with(b"\"") && !line.windows(3).any(|w| w == b"}\r\n")
}

/// A quoted string's contents with its escapes undone: `\\` is a backslash and `\"` a quote
/// (RFC 3501 §4.3), so Gmail's `"\\All"` is the label `\All`, not `\\All`.
fn unescape(quoted: &str) -> String {
    let mut out = String::with_capacity(quoted.len());
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            other => out.push(other),
        }
    }
    out
}

/// Which listed folder plays which role, as [`ProtoOutcome::Folders`] carries them in the
/// capabilities.
///
/// From the parsed listing and nothing else, so a role's path is the folder's path, however
/// the server wrote the name. `\All` is the archive, as it is on Gmail; `\Flagged` and
/// `\Important` are views, not mailboxes; and a container that holds no mail plays no role.
///
/// A role no folder's SPECIAL-USE attribute names (RFC 6154 is optional, and many servers send
/// none) goes to the folder named for it by the names clients agree on
/// ([`MailboxRole::from_folder_name`]), as Apple Mail and Thunderbird find them: `Sent`, `Junk`,
/// `Deleted Items`. Only at the top, or just under `INBOX` where Courier and Dovecot's older
/// layouts keep them, so a user's `Projects/Sent` is never taken for the account's; and only for
/// Sent, Drafts, Trash and Junk, since the archive is found another way ([`role_of`]).
///
/// [`ProtoOutcome::Folders`]: crate::ProtoOutcome::Folders
pub(crate) fn roles(listed: &[Folder]) -> FolderRoles {
    let mut named: Vec<(String, MailboxRole)> = listed
        .iter()
        .filter(|f| f.holds == Holds::Mail)
        .filter_map(|f| Some((f.path.clone(), role_of(f.special?)?)))
        .collect();
    for role in [
        MailboxRole::Sent,
        MailboxRole::Drafts,
        MailboxRole::Trash,
        MailboxRole::Spam,
    ] {
        if named.iter().any(|(_, has)| *has == role) {
            continue;
        }
        let guessed = listed
            .iter()
            .filter(|f| f.holds == Holds::Mail && f.special.is_none())
            .filter(|f| MailboxRole::from_folder_name(&f.path) == Some(role))
            .filter_map(|f| Some((standard_depth(f)?, f)))
            .min_by_key(|(depth, _)| *depth);
        if let Some((_, folder)) = guessed {
            named.push((folder.path.clone(), role));
        }
    }
    FolderRoles(named)
}

/// How far down a folder is where a standard folder is kept: 0 at the top, 1 just under
/// `INBOX`, `None` anywhere else.
fn standard_depth(folder: &Folder) -> Option<u8> {
    let Some(delimiter) = folder.delimiter else {
        return Some(0);
    };
    match folder.path.split_once(delimiter) {
        None => Some(0),
        Some((parent, rest))
            if parent.eq_ignore_ascii_case("INBOX") && !rest.contains(delimiter) =>
        {
            Some(1)
        }
        Some(_) => None,
    }
}

/// The role a mailbox with this special use plays in the capabilities.
fn role_of(special: SpecialUse) -> Option<MailboxRole> {
    match special {
        SpecialUse::All => Some(MailboxRole::Archive),
        // Not filed by role: an `\Archive` folder is reached through `ArchiveMeans`, and only
        // `\All` has ever named the archive role here.
        SpecialUse::Archive => None,
        other => other.role(),
    }
}

/// What an attribute says a mailbox is for, if it says anything.
fn special_use(attribute: &imap_proto::NameAttribute<'_>) -> Option<SpecialUse> {
    use imap_proto::NameAttribute as A;
    Some(match attribute {
        A::All => SpecialUse::All,
        A::Archive => SpecialUse::Archive,
        A::Drafts => SpecialUse::Drafts,
        A::Flagged => SpecialUse::Flagged,
        A::Junk => SpecialUse::Junk,
        A::Sent => SpecialUse::Sent,
        A::Trash => SpecialUse::Trash,
        // RFC 8457: a view of important mail, like Starred, and just as immovable.
        A::Extension(e) if e.eq_ignore_ascii_case("\\Important") => SpecialUse::Important,
        _ => return None,
    })
}

/// `\Noselect`, or `\NonExistent` (RFC 5258), which implies it.
fn holds_nothing(attribute: &imap_proto::NameAttribute<'_>) -> bool {
    match attribute {
        imap_proto::NameAttribute::NoSelect => true,
        imap_proto::NameAttribute::Extension(e) => e.eq_ignore_ascii_case("\\NonExistent"),
        _ => false,
    }
}

/// Whether a refusal of `work` means the server is already where the work would have put it.
///
/// Decided by the response code (RFC 5530), which is protocol, never by the prose after it:
/// `[ALREADYEXISTS]` to a create, `[NONEXISTENT]` to a delete or an unsubscribe. Anything else,
/// a bare `NO` included, is a real refusal, and the local change is undone.
pub(crate) fn already_so(work: &FolderWork, error: &ProtoError) -> bool {
    let ProtoError::Refused { text, .. } = error else {
        return false;
    };
    let Some(code) = response_code(text) else {
        return false;
    };
    match work {
        FolderWork::Create { .. } => code.eq_ignore_ascii_case("ALREADYEXISTS"),
        FolderWork::Delete { .. }
        | FolderWork::Subscribe {
            subscription: Subscription::Unsubscribed,
            ..
        } => code.eq_ignore_ascii_case("NONEXISTENT"),
        FolderWork::Rename { .. }
        | FolderWork::Subscribe {
            subscription: Subscription::Subscribed,
            ..
        } => false,
    }
}

/// The response code of a tagged reply: `ALREADYEXISTS` in `a3 NO [ALREADYEXISTS] Exists`.
///
/// Only in its place, directly after the status word. A bracket later on is prose.
fn response_code(text: &str) -> Option<&str> {
    let mut words = text.splitn(3, ' ');
    let _tag = words.next()?;
    let _status = words.next()?;
    let inside = words.next()?.strip_prefix('[')?;
    let end = inside.find([']', ' '])?;
    Some(&inside[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Refusal;

    fn refused(text: &str) -> ProtoError {
        ProtoError::Refused {
            kind: Refusal::Permanent,
            text: text.to_owned(),
        }
    }

    fn untagged(line: &str) -> crate::Untagged {
        crate::Untagged {
            during: 0,
            text: line.to_owned(),
            raw: format!("{line}\r\n").into_bytes(),
        }
    }

    #[test]
    fn the_inbox_is_the_inbox_by_name_whatever_attributes_it_carries() {
        let lines = [
            r#"* LIST (\Seen \Flagged \Deleted \Draft) "." "INBOX""#,
            r#"* LIST (\Flagged) "/" Starred"#,
        ];
        let listed = listing(
            &lines.map(untagged),
            mail_domain::id::account_id_from_uuid(uuid::uuid!(
                "00000000-0000-4000-8000-0000000000a1"
            )),
        );
        let special = |path: &str| {
            listed
                .iter()
                .find(|f| f.path == path)
                .and_then(|f| f.special)
        };
        assert_eq!(special("INBOX"), Some(SpecialUse::Inbox));
        assert_eq!(special("Starred"), Some(SpecialUse::Flagged));
    }

    /// A quoted name has its escapes undone; an atom's backslash is its own.
    #[test]
    fn a_quoted_name_is_unescaped_and_an_atom_is_kept() {
        let lines = [
            r#"* LIST (\HasNoChildren) "/" "\\All""#,
            r#"* LIST (\HasNoChildren) "/" "Say \"hi\"""#,
            r#"* LIST (\HasNoChildren) "/" Plain"#,
        ];
        let listed = listing(
            &lines.map(untagged),
            mail_domain::id::account_id_from_uuid(uuid::uuid!(
                "00000000-0000-4000-8000-0000000000a1"
            )),
        );
        let paths: Vec<&str> = listed.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["Plain", "Say \"hi\"", "\\All"]);
    }

    #[test]
    fn only_the_code_in_its_place_counts_as_already_done() {
        let create = FolderWork::Create {
            path: "Receipts".to_owned(),
        };
        const CASES: &[(&str, bool)] = &[
            ("a3 NO [ALREADYEXISTS] Mailbox exists", true),
            ("a3 NO [alreadyexists] Mailbox exists", true),
            // A different code, or the right word in the prose, is a real refusal.
            ("a3 NO [CANNOT] Invalid mailbox name", false),
            ("a3 NO Mailbox exists [ALREADYEXISTS]", false),
            ("a3 NO [ALREADYEXISTSX] Mailbox exists", false),
            ("a3 NO Mailbox exists", false),
        ];
        for (text, want) in CASES {
            assert_eq!(already_so(&create, &refused(text)), *want, "{text}");
        }
    }

    #[test]
    fn a_missing_folder_is_already_deleted_but_never_already_renamed() {
        let gone = refused("a3 NO [NONEXISTENT] No such mailbox");
        let delete = FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Refuse,
        };
        let rename = FolderWork::Rename {
            from: "Old".to_owned(),
            to: "New".to_owned(),
        };
        assert!(already_so(&delete, &gone));
        assert!(!already_so(&rename, &gone));
    }

    /// A role no SPECIAL-USE attribute names goes to the standard name at the top or just under
    /// `INBOX`; an attribute always wins, and a folder deeper down is never guessed at.
    #[test]
    fn a_server_without_special_use_has_its_standard_folders_found_by_name() {
        let account = mail_domain::id::account_id_from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-0000000000a1"
        ));
        type Case = (
            &'static str,
            &'static [&'static str],
            &'static [(&'static str, MailboxRole)],
        );
        let cases: &[Case] = &[
            (
                "none named",
                &[
                    r#"* LIST () "/" INBOX"#,
                    r#"* LIST () "/" "Sent Items""#,
                    r#"* LIST () "/" Drafts"#,
                    r#"* LIST () "/" "Deleted Items""#,
                    r#"* LIST () "/" "Junk E-mail""#,
                ],
                &[
                    ("INBOX", MailboxRole::Inbox),
                    ("Sent Items", MailboxRole::Sent),
                    ("Drafts", MailboxRole::Drafts),
                    ("Deleted Items", MailboxRole::Trash),
                    ("Junk E-mail", MailboxRole::Spam),
                ],
            ),
            (
                "under INBOX",
                &[
                    r#"* LIST () "." INBOX"#,
                    r#"* LIST () "." INBOX.Sent"#,
                    r#"* LIST () "." INBOX.Trash"#,
                ],
                &[
                    ("INBOX", MailboxRole::Inbox),
                    ("INBOX.Sent", MailboxRole::Sent),
                    ("INBOX.Trash", MailboxRole::Trash),
                ],
            ),
            (
                "an attribute wins",
                &[r#"* LIST () "/" Trash"#, r#"* LIST (\Trash) "/" Bin"#],
                &[("Bin", MailboxRole::Trash)],
            ),
            (
                "never deeper down",
                &[
                    r#"* LIST () "/" Projects"#,
                    r#"* LIST () "/" Projects/Sent"#,
                    r#"* LIST () "/" Projects/Spam"#,
                ],
                &[],
            ),
            (
                "the top over INBOX's",
                &[r#"* LIST () "/" INBOX/Spam"#, r#"* LIST () "/" Spam"#],
                &[("Spam", MailboxRole::Spam)],
            ),
            (
                "a container holds no mail",
                &[r#"* LIST (\Noselect) "/" Sent"#],
                &[],
            ),
        ];
        for (name, lines, want) in cases {
            let listed = listing(
                &lines.iter().map(|l| untagged(l)).collect::<Vec<_>>(),
                account.clone(),
            );
            let got = roles(&listed).0;
            let want: Vec<(String, MailboxRole)> =
                want.iter().map(|(p, r)| ((*p).to_owned(), *r)).collect();
            assert_eq!(got, want, "{name}");
        }
    }
}
