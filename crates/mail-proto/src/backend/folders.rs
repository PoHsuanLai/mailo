//! Mailbox management over IMAP: the walks `ProtoOp::Folder` becomes, reading a listing, and
//! telling a refusal that means "already done" from one that means "no".
//!
//! Separate from `imap.rs` because none of it touches a message, and that file is long enough.

use crate::imap::ImapCommand;
use crate::machine::ProtoError;
use crate::mutf7;
use mail_domain::{AccountId, Folder, FolderWork, Holds, NonEmpty, SpecialUse, Subscription};

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
        let path = mutf7::decode(&name);
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
        let special = name_attributes.iter().find_map(special_use).or_else(|| {
            path.eq_ignore_ascii_case("INBOX")
                .then_some(SpecialUse::Inbox)
        });
        let holds = if name_attributes.iter().any(holds_nothing) {
            Holds::FoldersOnly
        } else {
            Holds::Mail
        };
        listed.push(Folder {
            account,
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
}
