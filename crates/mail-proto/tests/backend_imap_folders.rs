//! `ImapBackend` managing mailboxes: `CREATE`, `RENAME`, `DELETE`, `SUBSCRIBE`, `UNSUBSCRIBE`,
//! and the `LIST`/`LSUB` walk that finds them — checked as bytes on the wire.
//!
//! Every transcript here is SYNTHETIC, written from RFC 3501 §6.3 and RFC 5530's response
//! codes rather than recorded. What they pin is the client's half: which commands, in which
//! order, with which names, and what a refusal is taken to mean.

mod common;

use common::replay;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{
    Backend, ImapAuth, ImapCommand, ImapSession, IoReady, Machine, Progress, ProtoError,
    ProtoOutcome, Refusal,
};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn caps(labels: ServerLabels) -> AccountCaps {
    AccountCaps {
        labels,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 5 },
        observed_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
    }
}

/// A password account, which is what every server but Gmail gets.
fn backend(labels: ServerLabels) -> ImapBackend {
    ImapBackend::new(
        ACCOUNT,
        caps(labels),
        Box::new(|auth: Authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(
                ImapAuth {
                    username: "ada@example.test".to_owned(),
                    credential: Credential::Password("hunter2".to_owned()),
                    sasl: vec![SaslMech::Plain],
                },
                all,
            )
        }),
    )
}

struct Driven {
    backend: ImapBackend,
    op: Option<ProtoOp>,
}

impl Machine for Driven {
    type Out = ProtoOutcome;
    fn start(&mut self) -> Progress<ProtoOutcome> {
        let op = self.op.take().expect("start called twice");
        self.backend.begin(op)
    }
    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
        self.backend.feed(ready)
    }
}

fn driven(work: FolderWork) -> Driven {
    Driven {
        backend: backend(ServerLabels::LocalOnly),
        op: Some(ProtoOp::Folder(work)),
    }
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

#[test]
fn creating_a_folder_creates_it_and_follows_it() {
    let walk = trace(concat!(
        "C: a002 CREATE \"Receipts\"\n",
        "S: a002 OK CREATE completed\n",
        "C: a003 SUBSCRIBE \"Receipts\"\n",
        "S: a003 OK SUBSCRIBE completed\n",
        "DONE\n",
    ));
    let outcome = replay(
        &mut driven(FolderWork::Create {
            path: "Receipts".to_owned(),
        }),
        &walk,
    )
    .unwrap();
    assert_eq!(outcome, ProtoOutcome::Applied);
}

/// The RFC 3501 §5.1.3 example: the name travels in modified UTF-7, and only on the wire.
#[test]
fn a_non_ascii_name_goes_out_in_modified_utf7() {
    let walk = trace(concat!(
        "C: a002 CREATE \"台北/日本語\"\n",
        "S: a002 OK CREATE completed\n",
        "C: a003 SUBSCRIBE \"台北/日本語\"\n",
        "S: a003 OK SUBSCRIBE completed\n",
        "DONE\n",
    ))
    .replace("台北/日本語", "&U,BTFw-/&ZeVnLIqe-");
    let outcome = replay(
        &mut driven(FolderWork::Create {
            path: "台北/日本語".to_owned(),
        }),
        &walk,
    )
    .unwrap();
    assert_eq!(outcome, ProtoOutcome::Applied);
}

/// `[ALREADYEXISTS]` to a create is the folder the user asked for, already there. Undoing the
/// local folder would make this client disagree with the server about something both have.
#[test]
fn a_create_refused_because_it_exists_is_done_not_failed() {
    let walk = trace(concat!(
        "C: a002 CREATE \"Receipts\"\n",
        "S: a002 NO [ALREADYEXISTS] Mailbox already exists\n",
        "DONE\n",
    ));
    let outcome = replay(
        &mut driven(FolderWork::Create {
            path: "Receipts".to_owned(),
        }),
        &walk,
    )
    .unwrap();
    assert_eq!(outcome, ProtoOutcome::Applied);
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

#[test]
fn a_rename_names_both_ends_on_the_wire() {
    let walk = trace(concat!(
        "C: a002 RENAME \"Work\" \"&ZeVnLIqe-\"\n",
        "S: a002 OK RENAME completed\n",
        "DONE\n",
    ));
    let outcome = replay(
        &mut driven(FolderWork::Rename {
            from: "Work".to_owned(),
            to: "日本語".to_owned(),
        }),
        &walk,
    )
    .unwrap();
    assert_eq!(outcome, ProtoOutcome::Applied);
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

/// Refusing to delete a folder with mail in it is asked of the server in the same walk as the
/// delete, and an empty answer lets it through.
#[test]
fn an_empty_folder_is_deleted_after_the_server_says_it_is_empty() {
    let walk = trace(concat!(
        "C: a002 STATUS \"Old\" (MESSAGES)\n",
        "S: * STATUS \"Old\" (MESSAGES 0)\n",
        "S: a002 OK STATUS completed\n",
        "C: a003 DELETE \"Old\"\n",
        "S: a003 OK DELETE completed\n",
        "DONE\n",
    ));
    let outcome = replay(
        &mut driven(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Refuse,
        }),
        &walk,
    )
    .unwrap();
    assert_eq!(outcome, ProtoOutcome::Applied);
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
    let mut gmail = Driven {
        backend: backend(ServerLabels::Supported),
        op: Some(ProtoOp::Folder(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Allow,
        })),
    };
    assert_eq!(replay(&mut gmail, &walk).unwrap(), ProtoOutcome::Applied);
    assert!(!walk.contains("EXPUNGE") && !walk.contains("\\Deleted"));
}

#[test]
fn a_folder_already_gone_is_already_deleted() {
    let walk = trace(concat!(
        "C: a002 DELETE \"Old\"\n",
        "S: a002 NO [NONEXISTENT] Mailbox doesn't exist\n",
        "DONE\n",
    ));
    let outcome = replay(
        &mut driven(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Allow,
        }),
        &walk,
    )
    .unwrap();
    assert_eq!(outcome, ProtoOutcome::Applied);
}

#[test]
fn subscribing_and_unsubscribing_are_one_command_each() {
    const CASES: &[(Subscription, &str)] = &[
        (Subscription::Subscribed, "SUBSCRIBE"),
        (Subscription::Unsubscribed, "UNSUBSCRIBE"),
    ];
    for (subscription, verb) in CASES {
        let walk = trace(&format!(
            "C: a002 {verb} \"Lists/rust\"\nS: a002 OK {verb} completed\nDONE\n"
        ));
        let outcome = replay(
            &mut driven(FolderWork::Subscribe {
                path: "Lists/rust".to_owned(),
                subscription: *subscription,
            }),
            &walk,
        )
        .unwrap();
        assert_eq!(outcome, ProtoOutcome::Applied, "{verb}");
    }
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
    let mut listing = Driven {
        backend: backend(ServerLabels::Supported),
        op: Some(ProtoOp::ListFolders),
    };
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
    let mut listing = Driven {
        backend: backend(ServerLabels::LocalOnly),
        op: Some(ProtoOp::ListFolders),
    };
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
