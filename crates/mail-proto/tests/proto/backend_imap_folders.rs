//! `ImapBackend` managing mailboxes: `CREATE`, `RENAME`, `DELETE`, `SUBSCRIBE`, `UNSUBSCRIBE`,
//! and the `LIST`/`LSUB` walk that finds them — checked as bytes on the wire.
//!
//! Every transcript here is SYNTHETIC, written from RFC 3501 §6.3 and RFC 5530's response
//! codes rather than recorded. What they pin is the client's half: which commands, in which
//! order, with which names, and what a refusal is taken to mean.

use crate::common;

use common::{Driven, password_backend, replay};
use mail_domain::*;
use mail_proto::backend::ImapBackend;
use mail_proto::{ProtoError, ProtoOutcome, Refusal};

fn caps(labels: ServerLabels) -> AccountCaps {
    AccountCaps {
        labels,
        archive: ArchiveMeans::LocalOnly,
        connections: ConnectionBudget { max: 5 },
        ..common::caps()
    }
}

/// A password account, which is what every server but Gmail gets.
fn backend(labels: ServerLabels) -> ImapBackend {
    password_backend(caps(labels))
}

fn driven(work: FolderWork) -> Driven<ImapBackend> {
    Driven::new(backend(ServerLabels::LocalOnly), ProtoOp::Folder(work))
}

/// The greeting and sign-in every walk below begins with.
const SIGN_IN: &str = concat!(
    "S: * OK [CAPABILITY IMAP4rev1] ready\n",
    "C: a001 LOGIN \"ada@example.test\" \"hunter2\"\n",
    "S: a001 OK logged in\n",
);

fn trace(rest: &str) -> String {
    format!("{SIGN_IN}{rest}")
}

/// `(what it shows, the work, the walk after sign-in)`, each of which the server takes.
fn applied() -> Vec<(&'static str, FolderWork, String)> {
    let create = |path: &str| FolderWork::Create {
        path: path.to_owned(),
    };
    vec![
        (
            "creating a folder creates it and follows it",
            create("Receipts"),
            concat!(
                "C: a002 CREATE \"Receipts\"\n",
                "S: a002 OK CREATE completed\n",
                "C: a003 SUBSCRIBE \"Receipts\"\n",
                "S: a003 OK SUBSCRIBE completed\n",
            )
            .to_owned(),
        ),
        // The RFC 3501 §5.1.3 example: the name travels in modified UTF-7, and only on the wire.
        (
            "a non-ASCII name goes out in modified UTF-7",
            create("台北/日本語"),
            concat!(
                "C: a002 CREATE \"&U,BTFw-/&ZeVnLIqe-\"\n",
                "S: a002 OK CREATE completed\n",
                "C: a003 SUBSCRIBE \"&U,BTFw-/&ZeVnLIqe-\"\n",
                "S: a003 OK SUBSCRIBE completed\n",
            )
            .to_owned(),
        ),
        // `[ALREADYEXISTS]` to a create is the folder the user asked for, already there.
        // Undoing the local folder would make this client disagree with the server about
        // something both have.
        (
            "a create refused because it exists is done, not failed",
            create("Receipts"),
            concat!(
                "C: a002 CREATE \"Receipts\"\n",
                "S: a002 NO [ALREADYEXISTS] Mailbox already exists\n",
            )
            .to_owned(),
        ),
        (
            "a rename names both ends on the wire",
            FolderWork::Rename {
                from: "Work".to_owned(),
                to: "日本語".to_owned(),
            },
            concat!(
                "C: a002 RENAME \"Work\" \"&ZeVnLIqe-\"\n",
                "S: a002 OK RENAME completed\n",
            )
            .to_owned(),
        ),
        // Refusing to delete a folder with mail in it is asked of the server in the same walk
        // as the delete, and an empty answer lets it through.
        (
            "an empty folder is deleted after the server says it is empty",
            FolderWork::Delete {
                path: "Old".to_owned(),
                non_empty: NonEmpty::Refuse,
            },
            concat!(
                "C: a002 STATUS \"Old\" (MESSAGES)\n",
                "S: * STATUS \"Old\" (MESSAGES 0)\n",
                "S: a002 OK STATUS completed\n",
                "C: a003 DELETE \"Old\"\n",
                "S: a003 OK DELETE completed\n",
            )
            .to_owned(),
        ),
        (
            "a folder already gone is already deleted",
            FolderWork::Delete {
                path: "Old".to_owned(),
                non_empty: NonEmpty::Allow,
            },
            concat!(
                "C: a002 DELETE \"Old\"\n",
                "S: a002 NO [NONEXISTENT] Mailbox doesn't exist\n",
            )
            .to_owned(),
        ),
    ]
    .into_iter()
    .chain(
        [
            (Subscription::Subscribed, "SUBSCRIBE"),
            (Subscription::Unsubscribed, "UNSUBSCRIBE"),
        ]
        .map(|(subscription, verb)| {
            (
                "subscribing and unsubscribing are one command each",
                FolderWork::Subscribe {
                    path: "Lists/rust".to_owned(),
                    subscription,
                },
                format!("C: a002 {verb} \"Lists/rust\"\nS: a002 OK {verb} completed\n"),
            )
        }),
    )
    .collect()
}

#[test]
fn each_folder_change_the_server_takes_is_applied() {
    for (shows, work, walk) in applied() {
        let outcome = replay(&mut driven(work), &trace(&format!("{walk}DONE\n"))).unwrap();
        assert_eq!(outcome, ProtoOutcome::Applied, "{shows}");
    }
}

/// Any other `NO` is a refusal the outbox undoes: permanent, so the folder is taken back here.
#[test]
fn a_create_the_server_refuses_fails_permanently() {
    let walk = trace(concat!(
        "C: a002 CREATE \"Receipts\"\n",
        "S: a002 NO [CANNOT] Invalid mailbox name\n",
        "FAIL Refused\n",
    ));
    let error = replay(
        &mut driven(FolderWork::Create {
            path: "Receipts".to_owned(),
        }),
        &walk,
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ProtoError::Refused {
                kind: Refusal::Permanent,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(matches!(error.retry(), Retry::Fatal(_)));
}

/// A rename onto a name that exists is not "already done": the folder being renamed is still
/// where it was.
#[test]
fn a_rename_refused_because_the_target_exists_fails() {
    let walk = trace(concat!(
        "C: a002 RENAME \"Work\" \"Jobs\"\n",
        "S: a002 NO [ALREADYEXISTS] Mailbox already exists\n",
        "FAIL Refused\n",
    ));
    replay(
        &mut driven(FolderWork::Rename {
            from: "Work".to_owned(),
            to: "Jobs".to_owned(),
        }),
        &walk,
    )
    .unwrap_err();
}

/// The server holds mail this client never synced: nothing is deleted. The trace ends at the
/// guard, and the harness fails any write it did not expect, so no `DELETE` went out.
#[test]
fn a_folder_the_server_says_holds_mail_is_not_deleted() {
    let walk = trace(concat!(
        "C: a002 STATUS \"Old\" (MESSAGES)\n",
        "S: * STATUS \"Old\" (MESSAGES 3)\n",
        "S: a002 OK STATUS completed\n",
        "FAIL Refused\n",
    ));
    let error = replay(
        &mut driven(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Refuse,
        }),
        &walk,
    )
    .unwrap_err();
    assert!(error.to_string().contains("3 message"), "{error}");
    assert!(matches!(error.retry(), Retry::Fatal(_)));
}

/// A server that does not say how much is in there is not taken to mean nothing is.
#[test]
fn a_status_with_no_count_is_not_an_empty_folder() {
    let walk = trace(concat!(
        "C: a002 STATUS \"Old\" (MESSAGES)\n",
        "S: a002 OK STATUS completed\n",
        "FAIL Refused\n",
    ));
    replay(
        &mut driven(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Refuse,
        }),
        &walk,
    )
    .unwrap_err();
}

/// Told explicitly, the guard is not asked. And on no server is anything flagged `\Deleted` or
/// expunged: `DELETE` removes the mailbox, which on Gmail is removing a label.
#[test]
fn deleting_with_its_mail_sends_delete_and_nothing_else() {
    let walk = trace(concat!(
        "C: a002 DELETE \"Old\"\n",
        "S: a002 OK DELETE completed\n",
        "DONE\n",
    ));
    let mut gmail = Driven::new(
        backend(ServerLabels::Supported),
        ProtoOp::Folder(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Allow,
        }),
    );
    assert_eq!(replay(&mut gmail, &walk).unwrap(), ProtoOutcome::Applied);
    assert!(!walk.contains("EXPUNGE") && !walk.contains("\\Deleted"));
}

/// A Gmail-shaped listing: special uses, a container, a label with a space, one in modified
/// UTF-7, and a subscription only `LSUB` reveals.
#[test]
fn a_listing_reports_every_folder_with_its_use_and_subscription() {
    let walk = trace(concat!(
        "C: a002 LIST \"\" \"*\"\n",
        "S: * LIST (\\HasNoChildren) \"/\" \"INBOX\"\n",
        "S: * LIST (\\HasChildren \\Noselect) \"/\" \"[Gmail]\"\n",
        "S: * LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/All Mail\"\n",
        "S: * LIST (\\HasNoChildren \\Important) \"/\" \"[Gmail]/Important\"\n",
        "S: * LIST (\\HasNoChildren \\Sent) \"/\" \"[Gmail]/Sent Mail\"\n",
        "S: * LIST (\\HasNoChildren \\Junk) \"/\" \"[Gmail]/Spam\"\n",
        "S: * LIST (\\HasNoChildren \\Trash) \"/\" \"[Gmail]/Trash\"\n",
        "S: * LIST (\\HasNoChildren) \"/\" \"Paid bills\"\n",
        "S: * LIST (\\HasNoChildren) \"/\" \"&ZeVnLIqe-\"\n",
        "S: a002 OK LIST completed\n",
        "C: a003 LSUB \"\" \"*\"\n",
        "S: * LSUB (\\HasNoChildren) \"/\" \"INBOX\"\n",
        "S: * LSUB (\\HasNoChildren) \"/\" \"&ZeVnLIqe-\"\n",
        "S: * LSUB () \"/\" \"Deleted long ago\"\n",
        "S: a003 OK LSUB completed\n",
        "DONE\n",
    ));
    let mut listing = Driven::new(backend(ServerLabels::Supported), ProtoOp::ListFolders);
    let ProtoOutcome::Folders { caps, listed } = replay(&mut listing, &walk).unwrap() else {
        panic!("a listing reports folders");
    };
    let row = |path: &str| {
        listed
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} was not listed: {listed:?}"))
    };

    assert_eq!(row("INBOX").special, Some(SpecialUse::Inbox));
    assert_eq!(row("[Gmail]").holds, Holds::FoldersOnly);
    assert_eq!(row("[Gmail]/All Mail").special, Some(SpecialUse::All));
    assert_eq!(
        row("[Gmail]/Important").special,
        Some(SpecialUse::Important)
    );
    assert_eq!(row("[Gmail]/Spam").special, Some(SpecialUse::Junk));
    assert_eq!(row("Paid bills").special, None);
    assert_eq!(row("Paid bills").delimiter, Some('/'));
    assert_eq!(row("Paid bills").subscription, Subscription::Unsubscribed);
    // Decoded for people, and followed because LSUB said so.
    assert_eq!(row("日本語").subscription, Subscription::Subscribed);
    // A subscription to nothing the server lists is not a folder.
    assert!(listed.iter().all(|f| f.path != "Deleted long ago"));
    assert_eq!(listed.len(), 9);
    // The roles still come with it, as they did before the listing did.
    assert_eq!(
        caps.folders.path(MailboxRole::Sent),
        Some("[Gmail]/Sent Mail")
    );
}

/// A name the server sends as an atom rather than a quoted string is still a name.
#[test]
fn a_listed_name_may_be_an_atom() {
    let walk = trace(concat!(
        "C: a002 LIST \"\" \"*\"\n",
        "S: * LIST () \".\" INBOX\n",
        "S: * LIST () \".\" INBOX.Archive\n",
        "S: a002 OK LIST completed\n",
        "C: a003 LSUB \"\" \"*\"\n",
        "S: a003 OK LSUB completed\n",
        "DONE\n",
    ));
    let mut listing = Driven::new(backend(ServerLabels::LocalOnly), ProtoOp::ListFolders);
    let ProtoOutcome::Folders { listed, .. } = replay(&mut listing, &walk).unwrap() else {
        panic!("a listing reports folders");
    };
    let paths: Vec<(&str, Option<char>)> = listed
        .iter()
        .map(|f| (f.path.as_str(), f.delimiter))
        .collect();
    assert_eq!(
        paths,
        vec![("INBOX", Some('.')), ("INBOX.Archive", Some('.'))]
    );
}

/// An Exchange Online-shaped listing: names sent as atoms where they need no quotes, `Inbox`
/// in mixed case, quoted names with spaces, a `\Noselect` parent, and the special-use flags
/// sent without `SPECIAL-USE` being advertised.
///
/// The roles were read from the text by taking whatever stood between the last two quotes,
/// which on `* LIST (\HasNoChildren \Drafts) "/" Drafts` is the delimiter: an account filed its
/// drafts in a folder called "/", and found no inbox, because `Inbox` was not `"INBOX"`.
#[test]
fn an_exchange_listing_gives_every_role_its_own_folder() {
    let walk = trace(concat!(
        "C: a002 LIST \"\" \"*\"\n",
        "S: * LIST (\\HasNoChildren) \"/\" Archive\n",
        "S: * LIST (\\Noselect \\HasChildren) \"/\" Calendar\n",
        "S: * LIST (\\HasNoChildren) \"/\" \"Calendar/Birthdays\"\n",
        "S: * LIST (\\HasNoChildren) \"/\" \"Conversation History\"\n",
        "S: * LIST (\\HasNoChildren \\Trash) \"/\" \"Deleted Items\"\n",
        "S: * LIST (\\HasNoChildren \\Drafts) \"/\" Drafts\n",
        "S: * LIST (\\Marked \\HasChildren) \"/\" Inbox\n",
        "S: * LIST (\\HasNoChildren) \"/\" Inbox/Receipts\n",
        "S: * LIST (\\HasNoChildren \\Junk) \"/\" \"Junk Email\"\n",
        "S: * LIST (\\HasNoChildren) \"/\" Outbox\n",
        "S: * LIST (\\HasNoChildren \\Sent) \"/\" \"Sent Items\"\n",
        "S: a002 OK LIST completed.\n",
        "C: a003 LSUB \"\" \"*\"\n",
        "S: * LSUB () \"/\" Inbox\n",
        "S: * LSUB () \"/\" \"Sent Items\"\n",
        "S: a003 OK LSUB completed.\n",
        "DONE\n",
    ));
    let mut listing = Driven::new(backend(ServerLabels::LocalOnly), ProtoOp::ListFolders);
    let ProtoOutcome::Folders { caps, listed } = replay(&mut listing, &walk).unwrap() else {
        panic!("a listing reports folders");
    };

    // Every role at its own folder, and no folder named after the delimiter.
    assert_eq!(caps.folders.path(MailboxRole::Inbox), Some("INBOX"));
    assert_eq!(caps.folders.path(MailboxRole::Drafts), Some("Drafts"));
    assert_eq!(caps.folders.path(MailboxRole::Sent), Some("Sent Items"));
    assert_eq!(caps.folders.path(MailboxRole::Trash), Some("Deleted Items"));
    assert_eq!(caps.folders.path(MailboxRole::Spam), Some("Junk Email"));
    assert!(
        caps.folders.0.iter().all(|(path, _)| path != "/"),
        "{:?}",
        caps.folders
    );
    assert!(listed.iter().all(|f| f.path != "/"), "{listed:?}");

    // `Inbox` is the inbox, under the one name RFC 3501 gives it, and followed as `LSUB` says.
    let inbox = listed
        .iter()
        .find(|f| f.special == Some(SpecialUse::Inbox))
        .unwrap_or_else(|| panic!("no inbox in {listed:?}"));
    assert_eq!(inbox.path, "INBOX");
    assert_eq!(inbox.subscription, Subscription::Subscribed);
    assert!(listed.iter().all(|f| f.path != "Inbox"), "{listed:?}");
    // Only the inbox itself is case-insensitive: a folder under it keeps the server's spelling.
    assert!(listed.iter().any(|f| f.path == "Inbox/Receipts"));

    let row = |path: &str| {
        listed
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} was not listed: {listed:?}"))
    };
    assert_eq!(row("Calendar").holds, Holds::FoldersOnly);
    assert_eq!(row("Conversation History").special, None);
    assert_eq!(row("Sent Items").subscription, Subscription::Subscribed);
    assert_eq!(row("Drafts").delimiter, Some('/'));
    assert_eq!(listed.len(), 11);
}

/// A name sent as a literal is a name like any other, and a role on it is the folder's.
#[test]
fn a_listed_name_may_be_a_literal() {
    let walk = trace(concat!(
        "C: a002 LIST \"\" \"*\"\n",
        "S: * LIST (\\HasNoChildren) \"/\" INBOX\n",
        "S: * LIST (\\HasNoChildren \\Sent) \"/\" {10}\n",
        "S: Sent \"Old\"\n",
        "S: a002 OK LIST completed\n",
        "C: a003 LSUB \"\" \"*\"\n",
        "S: a003 OK LSUB completed\n",
        "DONE\n",
    ));
    let mut listing = Driven::new(backend(ServerLabels::LocalOnly), ProtoOp::ListFolders);
    let ProtoOutcome::Folders { caps, listed } = replay(&mut listing, &walk).unwrap() else {
        panic!("a listing reports folders");
    };
    assert_eq!(caps.folders.path(MailboxRole::Sent), Some("Sent \"Old\""));
    assert!(
        listed.iter().any(|f| f.path == "Sent \"Old\""),
        "{listed:?}"
    );
}
