//! `folder::plan`: what a folder change is refused for, what it changes here, and its undo.

use mail_domain::folder::{FolderContents, FolderCtx, layered, plan};
use mail_domain::*;
use uuid::Uuid;

const ACCOUNT: AccountId = AccountId::from_uuid(Uuid::from_u128(0xacc0));

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

fn imap() -> Incoming {
    Incoming::Imap {
        host: "imap.example.test".to_owned(),
        port: 993,
        tls: Tls::Implicit,
    }
}

fn folder(path: &str, special: Option<SpecialUse>) -> Folder {
    Folder {
        account: ACCOUNT,
        path: path.to_owned(),
        delimiter: Some('/'),
        special,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    }
}

/// A Gmail-shaped account: the system folders, a container, and two of the user's own.
fn listing() -> Vec<Folder> {
    let mut gmail = folder("[Gmail]", None);
    gmail.holds = Holds::FoldersOnly;
    vec![
        folder("INBOX", None),
        gmail,
        folder("[Gmail]/All Mail", Some(SpecialUse::All)),
        folder("[Gmail]/Drafts", Some(SpecialUse::Drafts)),
        folder("[Gmail]/Important", Some(SpecialUse::Important)),
        folder("[Gmail]/Sent Mail", Some(SpecialUse::Sent)),
        folder("[Gmail]/Spam", Some(SpecialUse::Junk)),
        folder("[Gmail]/Starred", Some(SpecialUse::Flagged)),
        folder("[Gmail]/Trash", Some(SpecialUse::Trash)),
        folder("Archive", Some(SpecialUse::Archive)),
        folder("Work", None),
        folder("Work/2026", None),
        folder("Receipts", None),
    ]
}

fn label(name: &str) -> Label {
    Label {
        id: LabelId::from_uuid(Uuid::from_u128(0x1abe1)),
        account: ACCOUNT,
        name: name.to_owned(),
        color: None,
        origin: LabelOrigin::Provider,
    }
}

fn message(n: u128) -> MessageId {
    MessageId::from_uuid(Uuid::from_u128(n))
}

struct World {
    incoming: Incoming,
    caps: AccountCaps,
    folders: Vec<Folder>,
    labels: Vec<Label>,
    contents: FolderContents,
}

impl World {
    fn imap() -> Self {
        World {
            incoming: imap(),
            caps: caps(ServerLabels::LocalOnly),
            folders: listing(),
            labels: Vec::new(),
            contents: FolderContents::default(),
        }
    }

    fn gmail() -> Self {
        World {
            caps: caps(ServerLabels::Supported),
            labels: vec![label("Receipts")],
            ..World::imap()
        }
    }

    fn plan(&self, work: FolderWork) -> Result<Applied, FolderError> {
        plan(
            &work,
            &FolderCtx {
                account: ACCOUNT,
                incoming: &self.incoming,
                caps: &self.caps,
                folders: &self.folders,
                labels: &self.labels,
                contents: &self.contents,
            },
        )
    }
}

fn rename(from: &str, to: &str) -> FolderWork {
    FolderWork::Rename {
        from: from.to_owned(),
        to: to.to_owned(),
    }
}

fn delete(path: &str, non_empty: NonEmpty) -> FolderWork {
    FolderWork::Delete {
        path: path.to_owned(),
        non_empty,
    }
}

#[test]
fn a_special_use_folder_cannot_be_renamed_or_deleted() {
    const CASES: &[(&str, SpecialUse)] = &[
        ("INBOX", SpecialUse::Inbox),
        ("[Gmail]/All Mail", SpecialUse::All),
        ("[Gmail]/Drafts", SpecialUse::Drafts),
        ("[Gmail]/Important", SpecialUse::Important),
        ("[Gmail]/Sent Mail", SpecialUse::Sent),
        ("[Gmail]/Spam", SpecialUse::Junk),
        ("[Gmail]/Starred", SpecialUse::Flagged),
        ("[Gmail]/Trash", SpecialUse::Trash),
        ("Archive", SpecialUse::Archive),
    ];
    let world = World::imap();
    for (path, special) in CASES {
        let want = FolderError::Special {
            path: (*path).to_owned(),
            special: *special,
        };
        assert_eq!(
            world.plan(rename(path, "Elsewhere")).unwrap_err(),
            want,
            "rename {path}"
        );
        assert_eq!(
            world.plan(delete(path, NonEmpty::Allow)).unwrap_err(),
            want,
            "delete {path}"
        );
    }
}

/// The inbox by name, whatever the case and whether or not the server marked it.
#[test]
fn the_inbox_is_the_inbox_in_any_case() {
    let mut world = World::imap();
    world.folders = vec![folder("Inbox", None), folder("Work", None)];
    assert!(matches!(
        world.plan(rename("inbox", "Old mail")),
        Err(FolderError::Special {
            special: SpecialUse::Inbox,
            ..
        })
    ));
    assert_eq!(
        world
            .plan(FolderWork::Create {
                path: "INBOX".to_owned()
            })
            .unwrap_err(),
        FolderError::Exists("INBOX".to_owned())
    );
}

/// Moving `[Gmail]` would move Sent and Trash with it.
#[test]
fn a_folder_holding_a_special_use_folder_cannot_be_renamed() {
    let error = World::imap().plan(rename("[Gmail]", "Google")).unwrap_err();
    assert!(
        matches!(error, FolderError::Special { ref path, .. } if path.starts_with("[Gmail]/")),
        "{error:?}"
    );
}

#[test]
fn a_pop3_account_has_no_folders_to_change() {
    let mut world = World::imap();
    world.incoming = Incoming::Pop3 {
        host: "pop.example.test".to_owned(),
        port: 995,
        tls: Tls::Implicit,
        leave: LeaveOnServer::Keep,
    };
    for work in [
        FolderWork::Create {
            path: "New".to_owned(),
        },
        rename("Work", "Jobs"),
        delete("Receipts", NonEmpty::Allow),
        FolderWork::Subscribe {
            path: "Work".to_owned(),
            subscription: Subscription::Unsubscribed,
        },
    ] {
        assert_eq!(
            world.plan(work.clone()).unwrap_err(),
            FolderError::SingleMailbox,
            "{work:?}"
        );
    }
}

/// Mail this client holds is not deleted by accident: only with `NonEmpty::Allow`.
#[test]
fn a_folder_holding_mail_is_deleted_only_when_that_is_said() {
    let mut world = World::imap();
    world.contents = FolderContents {
        mapped: vec![message(1), message(2)],
        labelled: vec![message(2)],
    };
    assert_eq!(
        world
            .plan(delete("Receipts", NonEmpty::Refuse))
            .unwrap_err(),
        FolderError::NotEmpty {
            path: "Receipts".to_owned(),
            messages: 2,
        }
    );
    let applied = world.plan(delete("Receipts", NonEmpty::Allow)).unwrap();
    assert_eq!(
        applied.remote,
        Some(RemoteIntent::Folder(delete("Receipts", NonEmpty::Allow)))
    );
}

#[test]
fn an_empty_folder_is_deleted_and_the_server_is_asked_to_check() {
    let applied = World::imap()
        .plan(delete("Receipts", NonEmpty::Refuse))
        .unwrap();
    let receipts = MailboxRef {
        account: ACCOUNT,
        path: "Receipts".to_owned(),
    };
    assert_eq!(
        applied.forward.changes,
        vec![Change::FolderRemove(receipts)]
    );
    assert_eq!(
        applied.inverse.changes,
        vec![Change::FolderUpsert(folder("Receipts", None))]
    );
    // The refusal travels: the server re-checks, because this client may never have synced it.
    assert_eq!(
        applied.remote,
        Some(RemoteIntent::Folder(delete("Receipts", NonEmpty::Refuse)))
    );
}

#[test]
fn a_folder_with_folders_inside_is_not_deleted() {
    assert_eq!(
        World::imap()
            .plan(delete("Work", NonEmpty::Allow))
            .unwrap_err(),
        FolderError::HasChildren("Work".to_owned())
    );
}

/// On Gmail a folder is a label: deleting it takes the label off its messages and deletes
/// nothing, and the undo puts the label back on each of them.
#[test]
fn deleting_a_gmail_label_removes_the_label_and_keeps_the_mail() {
    let mut world = World::gmail();
    world.contents = FolderContents {
        mapped: Vec::new(),
        labelled: vec![message(1), message(2)],
    };
    let id = label("Receipts").id;
    let applied = world.plan(delete("Receipts", NonEmpty::Allow)).unwrap();
    assert_eq!(
        applied.forward.changes,
        vec![
            Change::MessageLabel(message(1), id, Membership::Out),
            Change::MessageLabel(message(2), id, Membership::Out),
            Change::LabelRemove(id),
            Change::FolderRemove(MailboxRef {
                account: ACCOUNT,
                path: "Receipts".to_owned(),
            }),
        ]
    );
    assert!(
        !applied
            .forward
            .changes
            .iter()
            .any(|c| matches!(c, Change::MessageDelete(_))),
        "no message is deleted"
    );
    assert_eq!(
        applied.inverse.changes,
        vec![
            Change::FolderUpsert(folder("Receipts", None)),
            Change::LabelUpsert(label("Receipts")),
            Change::MessageLabel(message(1), id, Membership::In),
            Change::MessageLabel(message(2), id, Membership::In),
        ]
    );
}

#[test]
fn creating_on_gmail_makes_a_label_to_file_under_at_once() {
    let applied = World::gmail()
        .plan(FolderWork::Create {
            path: "Travel".to_owned(),
        })
        .unwrap();
    let Some(Change::LabelUpsert(made)) = applied.forward.changes.get(1) else {
        panic!("no label: {:?}", applied.forward.changes);
    };
    assert_eq!(
        (made.name.as_str(), made.origin),
        ("Travel", LabelOrigin::Provider)
    );
    assert!(
        applied
            .inverse
            .changes
            .contains(&Change::LabelRemove(made.id))
    );
}

#[test]
fn creating_where_labels_are_local_makes_only_the_folder() {
    let applied = World::imap()
        .plan(FolderWork::Create {
            path: "Travel/2026".to_owned(),
        })
        .unwrap();
    assert_eq!(
        applied.forward.changes,
        vec![Change::FolderUpsert(folder("Travel/2026", None))]
    );
    assert_eq!(
        applied.inverse.changes,
        vec![Change::FolderRemove(MailboxRef {
            account: ACCOUNT,
            path: "Travel/2026".to_owned(),
        })]
    );
}

#[test]
fn a_name_that_exists_or_cannot_be_one_is_refused() {
    const BAD: &[&str] = &[
        "",
        "   ",
        "a\r\nb",
        "tab\there",
        "wild*",
        "per%cent",
        "/lead",
        "trail/",
        "dou//ble",
    ];
    let world = World::imap();
    for name in BAD {
        let error = world
            .plan(FolderWork::Create {
                path: (*name).to_owned(),
            })
            .unwrap_err();
        assert!(
            matches!(error, FolderError::BadName { .. }),
            "{name:?}: {error:?}"
        );
    }
    assert_eq!(
        world
            .plan(FolderWork::Create {
                path: "Receipts".to_owned()
            })
            .unwrap_err(),
        FolderError::Exists("Receipts".to_owned())
    );
    assert_eq!(
        world.plan(rename("Receipts", "Work")).unwrap_err(),
        FolderError::Exists("Work".to_owned())
    );
    assert_eq!(
        world.plan(rename("Nowhere", "Somewhere")).unwrap_err(),
        FolderError::Unknown("Nowhere".to_owned())
    );
}

#[test]
fn a_folder_cannot_be_moved_inside_itself() {
    assert!(matches!(
        World::imap().plan(rename("Work", "Work/Inner")),
        Err(FolderError::BadName { .. })
    ));
}

#[test]
fn a_rename_is_undone_by_the_rename_back() {
    let applied = World::imap().plan(rename("Work", "Jobs")).unwrap();
    let at = |path: &str| MailboxRef {
        account: ACCOUNT,
        path: path.to_owned(),
    };
    assert_eq!(
        applied.forward.changes,
        vec![Change::FolderRename {
            from: at("Work"),
            to: "Jobs".to_owned(),
            delimiter: Some('/'),
        }]
    );
    assert_eq!(
        applied.inverse.changes,
        vec![Change::FolderRename {
            from: at("Jobs"),
            to: "Work".to_owned(),
            delimiter: Some('/'),
        }]
    );
}

#[test]
fn following_a_folder_is_undone_by_its_prior_state() {
    let applied = World::imap()
        .plan(FolderWork::Subscribe {
            path: "Receipts".to_owned(),
            subscription: Subscription::Unsubscribed,
        })
        .unwrap();
    let mut unfollowed = folder("Receipts", None);
    unfollowed.subscription = Subscription::Unsubscribed;
    assert_eq!(
        applied.forward.changes,
        vec![Change::FolderUpsert(unfollowed)]
    );
    assert_eq!(
        applied.inverse.changes,
        vec![Change::FolderUpsert(folder("Receipts", None))]
    );
}

/// A listing taken before the server heard about queued work does not undo it here.
#[test]
fn queued_work_is_laid_over_a_fresh_listing() {
    let listed = vec![
        folder("INBOX", None),
        folder("Work", None),
        folder("Work/2026", None),
        folder("Old", None),
    ];
    let pending = vec![
        FolderWork::Create {
            path: "New".to_owned(),
        },
        rename("Work", "Jobs"),
        delete("Old", NonEmpty::Refuse),
        FolderWork::Subscribe {
            path: "Jobs/2026".to_owned(),
            subscription: Subscription::Unsubscribed,
        },
    ];
    let folders = layered(ACCOUNT, listed, &pending);
    let paths: Vec<(&str, Subscription)> = folders
        .iter()
        .map(|f| (f.path.as_str(), f.subscription))
        .collect();
    assert_eq!(
        paths,
        vec![
            ("INBOX", Subscription::Subscribed),
            ("Jobs", Subscription::Subscribed),
            ("Jobs/2026", Subscription::Unsubscribed),
            ("New", Subscription::Subscribed),
        ]
    );
}
