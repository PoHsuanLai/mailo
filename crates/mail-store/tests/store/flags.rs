//! A flag's colour, and labels kept as IMAP keywords.
//!
//! The colour is a column of its own beside the star (migration 0045): written by an op, read
//! from a sync's `flag_colors`, and held over a sync while the change that set it is still on its
//! way to the server. Labels on an account whose server keeps them as keywords are matched to the
//! keyword their name makes, and Thunderbird's tags arrive in Thunderbird's colours.

use crate::support::*;
use mail_domain::*;
use mail_store::{Dispatch, SqliteStore, Store};

/// Message 1 in the inbox at UID 10.
fn delivered(store: &dyn Store) -> Message {
    let m = message(1);
    deliver(store, &m, imap("INBOX", 10));
    m
}

/// The colour message `m` and its conversation wear, and whether they are flagged.
fn worn(store: &dyn Store, m: &Message) -> (Star, Hue, Star, Hue) {
    let held = store.message(m.id).unwrap();
    let summary = store.thread(m.thread).unwrap().summary;
    (held.star, held.flag, summary.star, summary.flag)
}

#[test]
fn a_colour_is_kept_on_the_message_and_its_conversation() {
    let found = Both::new().agree(|store| {
        let m = delivered(store);
        let flag = patch(vec![
            Change::MessageStar(m.id, Star::Starred),
            Change::MessageFlag(m.id, Hue::Orange),
        ]);
        store.apply(acct_account(), &flag).unwrap();
        worn(store, &m)
    });
    assert_eq!(
        found,
        (Star::Starred, Hue::Orange, Star::Starred, Hue::Orange)
    );
}

#[test]
fn a_sync_says_what_colour_a_message_is() {
    let found = Both::new().agree(|store| {
        let m = delivered(store);
        let batch = Ingest {
            flags: vec![(imap("INBOX", 10), ReadState::Read, Star::Starred)],
            flag_colors: vec![(imap("INBOX", 10), Hue::Purple)],
            ..ingest("INBOX")
        };
        store.ingest(acct_account(), batch).unwrap();
        worn(store, &m)
    });
    assert_eq!(
        found,
        (Star::Starred, Hue::Purple, Star::Starred, Hue::Purple)
    );
}

/// A colour set here and not yet sent stands over a sync that still reads the old one, as a star
/// does, and is what is sent.
#[test]
fn a_colour_on_its_way_to_the_server_outlives_a_sync() {
    let found = Both::new().agree(|store| {
        let m = delivered(store);
        let undo = patch(vec![
            Change::MessageStar(m.id, Star::Unstarred),
            Change::MessageFlag(m.id, Hue::Red),
        ]);
        let intent = RemoteIntent::SetFlag {
            messages: vec![m.id],
            hue: Hue::Green,
        };
        let queued = store
            .enqueue(acct_account(), intent, &undo, at(0))
            .unwrap()
            .expect("queued");
        store
            .apply(
                acct_account(),
                &patch(vec![
                    Change::MessageStar(m.id, Star::Starred),
                    Change::MessageFlag(m.id, Hue::Green),
                ]),
            )
            .unwrap();
        let stale = Ingest {
            flags: vec![(imap("INBOX", 10), ReadState::Unread, Star::Unstarred)],
            flag_colors: vec![(imap("INBOX", 10), Hue::Red)],
            ..ingest("INBOX")
        };
        store.ingest(acct_account(), stale).unwrap();
        let sent = match store.outbox_dispatch(queued).unwrap() {
            Dispatch::Send(op) => op,
            other => panic!("{other:?}"),
        };
        (worn(store, &m), sent)
    });
    assert_eq!(
        found,
        (
            (Star::Starred, Hue::Green, Star::Starred, Hue::Green),
            ProtoOp::SetFlag {
                remotes: vec![imap("INBOX", 10)],
                hue: Hue::Green,
            }
        )
    );
}

fn caps(labels: ServerLabels) -> AccountCaps {
    AccountCaps {
        labels,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: at(0),
    }
}

fn label_named(store: &SqliteStore, name: &str) -> Option<Label> {
    store
        .labels(acct_account())
        .unwrap()
        .into_iter()
        .find(|label| label.name == name)
}

fn user_label(store: &SqliteStore, name: &str) -> Label {
    let label = Label {
        id: LabelId::generate(),
        account: acct_account(),
        name: name.to_owned(),
        color: None,
        origin: LabelOrigin::User,
    };
    store
        .apply(
            acct_account(),
            &patch(vec![Change::LabelUpsert(label.clone())]),
        )
        .unwrap();
    label
}

/// A keyword is matched to the label whose name makes it, so `Project_X` is `Project X` and no
/// second label; Thunderbird's `$label4` is To Do, in Thunderbird's blue.
#[test]
fn keywords_read_back_as_the_labels_that_wrote_them() {
    let store = sqlite();
    store
        .put_caps(acct_account(), &caps(ServerLabels::Keywords), at(0))
        .unwrap();
    let m = delivered(&store);
    let project = user_label(&store, "Project X");
    let batch = Ingest {
        label_names: vec![(
            imap("INBOX", 10),
            vec!["Project_X".to_owned(), "To Do".to_owned()],
        )],
        ..ingest("INBOX")
    };
    store.ingest(acct_account(), batch).unwrap();

    let to_do = label_named(&store, "To Do").expect("To Do is made");
    assert_eq!(to_do.color.as_deref(), Some("blue"));
    assert_eq!(to_do.origin, LabelOrigin::User);
    assert!(label_named(&store, "Project_X").is_none(), "a second label");
    let mut held = store.message(m.id).unwrap().labels;
    held.sort();
    let mut want = vec![project.id, to_do.id];
    want.sort();
    assert_eq!(held, want);
}

/// A label whose name makes no keyword is this computer's alone, and a sync that cannot carry it
/// does not take it away.
#[test]
fn a_label_with_no_keyword_outlives_a_sync() {
    let store = sqlite();
    store
        .put_caps(acct_account(), &caps(ServerLabels::Keywords), at(0))
        .unwrap();
    let m = delivered(&store);
    let local = user_label(&store, "工作");
    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageLabel(m.id, local.id, Membership::In)]),
        )
        .unwrap();
    let batch = Ingest {
        label_names: vec![(imap("INBOX", 10), Vec::new())],
        ..ingest("INBOX")
    };
    store.ingest(acct_account(), batch).unwrap();
    assert_eq!(store.message(m.id).unwrap().labels, vec![local.id]);
}

/// Labels made before the server kept them are sent to it once it does, so the first sync after
/// does not read their absence there as their removal.
#[test]
fn labels_held_before_keywords_are_sent_when_the_server_keeps_them() {
    let store = sqlite();
    store
        .put_caps(acct_account(), &caps(ServerLabels::LocalOnly), at(0))
        .unwrap();
    let m = delivered(&store);
    let work = user_label(&store, "Work");
    store
        .apply(
            acct_account(),
            &patch(vec![Change::MessageLabel(m.id, work.id, Membership::In)]),
        )
        .unwrap();
    assert!(store.outbox_due(acct_account(), at(1)).unwrap().is_empty());

    store
        .put_caps(acct_account(), &caps(ServerLabels::Keywords), at(1))
        .unwrap();
    let due = store.outbox_due(acct_account(), at(2)).unwrap();
    assert_eq!(due.len(), 1, "{due:?}");
    match store.outbox_dispatch(due[0].id).unwrap() {
        Dispatch::Send(ProtoOp::SetLabels {
            remotes,
            add,
            remove,
        }) => {
            assert_eq!(remotes, vec![imap("INBOX", 10)]);
            assert_eq!(add, vec!["Work".to_owned()]);
            assert!(remove.is_empty());
        }
        other => panic!("{other:?}"),
    }

    // Seen once: the server's empty list does not take Work away while it is on its way.
    let batch = Ingest {
        label_names: vec![(imap("INBOX", 10), Vec::new())],
        ..ingest("INBOX")
    };
    store.ingest(acct_account(), batch).unwrap();
    assert_eq!(store.message(m.id).unwrap().labels, vec![work.id]);
    // And told again, nothing more is queued.
    store
        .put_caps(acct_account(), &caps(ServerLabels::Keywords), at(3))
        .unwrap();
    assert_eq!(store.outbox_due(acct_account(), at(4)).unwrap().len(), 1);
}
