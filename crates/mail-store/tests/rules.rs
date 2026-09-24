//! Rules: kept in both stores alike, and acting on mail the way the user's own hand would.
//!
//! Every behavioural test runs against both [`Store`] implementations through one generic
//! scenario and compares what each did — the outbox it left and the messages' state — because
//! `mail_store::rules` is written over the trait and a divergence between the stores would show
//! up as a rule that files mail on one and not the other.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::rules::{self, Ran};
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const OTHER: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));

fn sqlite() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    for (account, address) in [(ACCOUNT, "me@example.test"), (OTHER, "also@example.test")] {
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, '{}', datetime('now'))",
                [account.to_string(), address.to_owned()],
            )
            .unwrap();
    }
    (store, dir)
}

fn caps(labels: ServerLabels, archive: ArchiveMeans) -> AccountCaps {
    AccountCaps {
        labels,
        threads: ServerThreads::ProviderId,
        watch: WatchMode::Idle,
        archive,
        folders: FolderRoles::default(),
        condstore: Condstore::Supported,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: at(0),
    }
}

fn gmail() -> AccountCaps {
    caps(ServerLabels::Supported, ArchiveMeans::DropInbox)
}

fn folders() -> AccountCaps {
    caps(
        ServerLabels::LocalOnly,
        ArchiveMeans::MoveToFolder("Archive".to_owned()),
    )
}

fn rule(position: u32, name: &str, filter: Filter, actions: Vec<RuleAction>) -> Rule {
    Rule {
        id: RuleId::generate(),
        account: ACCOUNT,
        name: name.to_owned(),
        position,
        state: RuleState::Enabled,
        filter,
        actions,
        after: AfterMatch::Continue,
    }
}

fn from(s: &str) -> Filter {
    Filter::From(TextMatch::Contains(s.to_owned()))
}

/// One message from `sender`, in its own thread, as the server would hand it over.
fn arriving(n: u32, sender: &str, mailbox: MailboxRole, raw: BlobId) -> Fetched {
    let key = format!("m{n}@example.test");
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(key.clone()),
        date: at(i64::from(n)),
        from: Address {
            name: None,
            email: sender.to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: format!("Message {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(key.clone()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox,
        labels: vec![],
        body: Body::Present {
            text: Some(format!("body of message {n}")),
            raw,
        },
        attachments: vec![],
    };
    Fetched {
        remote: RemoteRef::Imap {
            mailbox: "INBOX".to_owned(),
            uidvalidity: 1,
            uid: n,
        },
        key: MessageKey::Rfc(key),
        raw,
        message,
    }
}

fn ingest(messages: Vec<Fetched>) -> Ingest {
    Ingest {
        mailbox: MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        validity: UidValidity::Same,
        cursor: None,
        messages,
        flags: vec![],
        labels: vec![],
        label_names: vec![],
        gone: vec![],
    }
}

/// What a sync pass reports as arrived: the messages the ingest stored for the first time.
fn arrived(patch: &Patch) -> Vec<MessageId> {
    patch
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::MessageUpsert(m) => Some(m.id),
            _ => None,
        })
        .collect()
}

fn outbox<S: Store>(store: &S) -> Vec<ProtoOp> {
    store
        .outbox_due(ACCOUNT, at(1_000_000))
        .unwrap()
        .into_iter()
        .map(|e| e.op)
        .collect()
}

fn names(ran: &Ran) -> Vec<Vec<String>> {
    ran.acted.iter().map(|(_, names)| names.clone()).collect()
}

/// Where a message is, whether read, whether starred, and its labels by name.
type State = (MailboxRole, ReadState, Star, Vec<String>);

/// The state a rule leaves a message in, with labels by name so two stores' ids need not agree.
fn state<S: Store>(store: &S, id: MessageId) -> State {
    let m = store.message(id).unwrap();
    let labels = store.labels(ACCOUNT).unwrap();
    let mut named: Vec<String> = m
        .labels
        .iter()
        .filter_map(|l| labels.iter().find(|x| x.id == *l).map(|x| x.name.clone()))
        .collect();
    named.sort();
    (m.mailbox, m.read, m.star, named)
}

#[test]
fn rules_are_kept_in_order_with_one_name_each_per_account() {
    let (sqlite, _dir) = sqlite();
    let memory = MemoryStore::new();
    let stores: [&dyn Store; 2] = [&sqlite, &memory];
    for store in stores {
        let b = rule(2, "b", from("x"), vec![RuleAction::Star]);
        let a = rule(2, "a", Filter::All, vec![RuleAction::MarkRead]);
        let first = rule(1, "z first", Filter::Nothing, vec![]);
        for r in [&b, &a, &first] {
            store.put_rule(r).unwrap();
        }
        let listed: Vec<String> = store
            .rules(ACCOUNT)
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(listed, ["z first", "a", "b"]);

        // A second rule by an existing name is refused; the same rule saved again is not.
        let clash = rule(3, "a", Filter::All, vec![]);
        assert!(matches!(
            store.put_rule(&clash),
            Err(StoreError::RuleNameTaken(name)) if name == "a"
        ));
        let edited = Rule {
            state: RuleState::Disabled,
            ..a.clone()
        };
        store.put_rule(&edited).unwrap();
        assert_eq!(store.rules(ACCOUNT).unwrap()[1].state, RuleState::Disabled);
        // Another account may use the name.
        store
            .put_rule(&Rule {
                account: OTHER,
                ..clash
            })
            .unwrap();
        assert_eq!(store.rules(OTHER).unwrap().len(), 1);

        store.delete_rule(b.id).unwrap();
        assert!(matches!(
            store.delete_rule(b.id),
            Err(StoreError::NoRule(_))
        ));
        assert_eq!(store.rules(ACCOUNT).unwrap().len(), 2);
    }
}

#[test]
fn a_vacation_reply_is_kept_replaced_and_cleared() {
    let (sqlite, _dir) = sqlite();
    let memory = MemoryStore::new();
    let stores: [&dyn Store; 2] = [&sqlite, &memory];
    for store in stores {
        assert_eq!(store.vacation(ACCOUNT).unwrap(), None);
        let away = Vacation {
            account: ACCOUNT,
            subject: "Away".to_owned(),
            body: "Back on Monday.".to_owned(),
            days: Vacation::DEFAULT_DAYS,
            addresses: vec!["me@example.test".to_owned()],
            from: None,
            during: DateRange {
                from: Some(at(0)),
                to: Some(at(86_400)),
            },
        };
        store.put_vacation(ACCOUNT, Some(&away), at(0)).unwrap();
        assert_eq!(store.vacation(ACCOUNT).unwrap(), Some(away.clone()));
        let longer = Vacation { days: 14, ..away };
        store.put_vacation(ACCOUNT, Some(&longer), at(1)).unwrap();
        assert_eq!(store.vacation(ACCOUNT).unwrap(), Some(longer));
        assert_eq!(store.vacation(OTHER).unwrap(), None);
        store.put_vacation(ACCOUNT, None, at(2)).unwrap();
        assert_eq!(store.vacation(ACCOUNT).unwrap(), None);
    }
}

/// What the arrival scenario saw, for comparing the two stores.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    first: Vec<Vec<String>>,
    queued: usize,
    ops: Vec<ProtoOp>,
    bank: State,
    friend: State,
    again: Vec<Vec<String>>,
    outbox_after_again: usize,
}

/// Two messages arrive on a Gmail account; one matches. The rule acts once, as the user would,
/// and a second pass over the same mail — a resync, the same message under a new UID — does
/// nothing, because nothing arrived.
fn arrival<S: Store>(store: &S, raw: BlobId) -> Seen {
    store
        .put_rule(&rule(
            1,
            "Bills",
            from("bank.example"),
            vec![
                RuleAction::MarkRead,
                RuleAction::Label("Bills".to_owned()),
                RuleAction::Archive,
            ],
        ))
        .unwrap();
    let bank = arriving(1, "statements@bank.example", MailboxRole::Inbox, raw);
    let friend = arriving(2, "ada@example.test", MailboxRole::Inbox, raw);
    let (bank_id, friend_id) = (bank.message.id, friend.message.id);
    let patch = store
        .ingest(ACCOUNT, ingest(vec![bank.clone(), friend.clone()]))
        .unwrap();
    let new = arrived(&patch);
    assert_eq!(new.len(), 2, "both are new");

    let ran = rules::at_arrival(store, ACCOUNT, &gmail(), &new, at(10)).unwrap();
    let ops = outbox(store);

    // The same messages again: stored already, so they did not arrive.
    let patch = store.ingest(ACCOUNT, ingest(vec![bank, friend])).unwrap();
    let again = rules::at_arrival(store, ACCOUNT, &gmail(), &arrived(&patch), at(20)).unwrap();

    Seen {
        first: names(&ran),
        queued: ran.queued,
        bank: state(store, bank_id),
        friend: state(store, friend_id),
        ops,
        again: names(&again),
        outbox_after_again: outbox(store).len(),
    }
}

#[test]
fn a_rule_acts_once_on_arriving_mail_and_the_server_is_told_as_the_user_would_tell_it() {
    let (sqlite, _dir) = sqlite();
    let raw = sqlite.blobs().put(&sqlite.connection(), b"raw").unwrap();
    let seen = arrival(&sqlite, raw);

    assert_eq!(seen.first, vec![vec!["Bills".to_owned()]]);
    assert_eq!(seen.queued, 3);
    assert_eq!(
        seen.bank,
        (
            MailboxRole::Archive,
            ReadState::Read,
            Star::Unstarred,
            vec!["Bills".to_owned()]
        )
    );
    assert_eq!(
        seen.friend,
        (
            MailboxRole::Inbox,
            ReadState::Unread,
            Star::Unstarred,
            vec![]
        )
    );
    let bank_remote = RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 1,
        uid: 1,
    };
    assert_eq!(
        seen.ops,
        vec![
            ProtoOp::SetFlags {
                remotes: vec![bank_remote.clone()],
                read: Some(ReadState::Read),
                star: None,
            },
            ProtoOp::SetLabels {
                remotes: vec![bank_remote.clone()],
                add: vec!["Bills".to_owned()],
                remove: vec![],
            },
            // Archive on Gmail: `SetMailbox` to Archive, which the backend sends as the Inbox
            // label removed.
            ProtoOp::SetMailbox {
                remotes: vec![bank_remote],
                role: MailboxRole::Archive,
            },
        ]
    );
    assert!(seen.again.is_empty(), "nothing arrived the second time");
    assert_eq!(seen.outbox_after_again, 3, "and nothing more was queued");

    // The in-memory store did exactly the same.
    let memory = MemoryStore::new();
    assert_eq!(arrival(&memory, BlobId::generate()), seen);
}

/// Mail that was here before a rule existed is left alone at arrival, and `run_now` reaches it,
/// in batches, on both stores alike.
fn backlog<S: Store>(store: &S, raw: BlobId) -> (Vec<Vec<String>>, Vec<usize>, Ran, Vec<ProtoOp>) {
    let old: Vec<Fetched> = (1..=3)
        .map(|n| arriving(n, "news@lists.example", MailboxRole::Inbox, raw))
        .chain([arriving(4, "ada@example.test", MailboxRole::Inbox, raw)])
        // Sent by the user: a rule is about mail received.
        .chain([arriving(5, "news@lists.example", MailboxRole::Sent, raw)])
        .collect();
    store.ingest(ACCOUNT, ingest(old)).unwrap();

    let news = rule(
        1,
        "News",
        from("lists.example"),
        vec![RuleAction::File("Newsletters".to_owned())],
    );
    store.put_rule(&news).unwrap();

    let fresh = arriving(6, "news@lists.example", MailboxRole::Inbox, raw);
    let patch = store.ingest(ACCOUNT, ingest(vec![fresh])).unwrap();
    let at_arrival =
        rules::at_arrival(store, ACCOUNT, &folders(), &arrived(&patch), at(10)).unwrap();

    let mut batches = Vec::new();
    let ran = rules::run_now(store, &folders(), &news, 2, at(20), &mut |batch| {
        batches.push(batch.examined)
    })
    .unwrap();
    (names(&at_arrival), batches, ran, outbox(store))
}

#[test]
fn run_now_reaches_mail_that_was_already_here_in_batches() {
    let (sqlite, _dir) = sqlite();
    let raw = sqlite.blobs().put(&sqlite.connection(), b"raw").unwrap();
    let (at_arrival, batches, ran, ops) = backlog(&sqlite, raw);

    assert_eq!(
        at_arrival,
        vec![vec!["News".to_owned()]],
        "only the new one"
    );
    // Six conversations, two a batch, newest first; the Sent one is looked past, not at.
    assert_eq!(batches, vec![1, 2, 2]);
    assert_eq!(ran.examined, 5);
    // Four newsletters match. The one filed at arrival is already where the rule puts it, so
    // only the three old ones had anything to queue.
    assert_eq!(ran.acted.len(), 4);
    assert_eq!(ran.queued, 3);
    let filed: Vec<&ProtoOp> = ops
        .iter()
        .filter(|op| matches!(op, ProtoOp::File { folder, .. } if folder == "Newsletters"))
        .collect();
    assert_eq!(filed.len(), 4, "{ops:?}");

    let memory = MemoryStore::new();
    let (m_arrival, m_batches, m_ran, m_ops) = backlog(&memory, BlobId::generate());
    assert_eq!(
        (m_arrival, m_batches, names(&m_ran), m_ran.queued, m_ops),
        (at_arrival, batches, names(&ran), ran.queued, ops)
    );
}

/// Rules run in order against the message as the ones before left it; `stop` ends it.
fn chain<S: Store>(store: &S, raw: BlobId) -> (Vec<Vec<String>>, State) {
    let mut first = rule(1, "read it", from("ada"), vec![RuleAction::MarkRead]);
    first.after = AfterMatch::Continue;
    let second = rule(
        2,
        "star what is read",
        Filter::Read(ReadState::Read),
        vec![RuleAction::Star],
    );
    let mut third = rule(3, "stop here", Filter::All, vec![]);
    third.after = AfterMatch::Stop;
    let never = rule(4, "never reached", Filter::All, vec![RuleAction::Trash]);
    let mut off = rule(0, "disabled", Filter::All, vec![RuleAction::Spam]);
    off.state = RuleState::Disabled;
    for r in [&never, &third, &second, &first, &off] {
        store.put_rule(r).unwrap();
    }
    let m = arriving(1, "ada@example.test", MailboxRole::Inbox, raw);
    let id = m.message.id;
    let patch = store.ingest(ACCOUNT, ingest(vec![m])).unwrap();
    let ran = rules::at_arrival(store, ACCOUNT, &folders(), &arrived(&patch), at(10)).unwrap();
    (names(&ran), state(store, id))
}

#[test]
fn a_later_rule_sees_what_an_earlier_one_did_and_stop_means_stop() {
    let (sqlite, _dir) = sqlite();
    let raw = sqlite.blobs().put(&sqlite.connection(), b"raw").unwrap();
    let seen = chain(&sqlite, raw);
    assert_eq!(
        seen.0,
        vec![vec![
            "read it".to_owned(),
            "star what is read".to_owned(),
            "stop here".to_owned()
        ]]
    );
    assert_eq!(
        seen.1,
        (MailboxRole::Inbox, ReadState::Read, Star::Starred, vec![])
    );
    assert_eq!(chain(&MemoryStore::new(), BlobId::generate()), seen);
}

#[test]
fn filing_on_a_server_with_folders_queues_one_move_into_the_folder() {
    let (store, _dir) = sqlite();
    let raw = store.blobs().put(&store.connection(), b"raw").unwrap();
    store
        .put_rule(&rule(
            1,
            "Projects",
            Filter::Subject(TextMatch::Contains("message".to_owned())),
            vec![RuleAction::File("Projects/2026".to_owned())],
        ))
        .unwrap();
    let m = arriving(3, "ada@example.test", MailboxRole::Inbox, raw);
    let id = m.message.id;
    let patch = store.ingest(ACCOUNT, ingest(vec![m])).unwrap();
    rules::at_arrival(&store, ACCOUNT, &folders(), &arrived(&patch), at(10)).unwrap();
    assert_eq!(
        outbox(&store),
        vec![ProtoOp::File {
            remotes: vec![RemoteRef::Imap {
                mailbox: "INBOX".to_owned(),
                uidvalidity: 1,
                uid: 3,
            }],
            folder: "Projects/2026".to_owned(),
        }]
    );
    assert_eq!(
        state(&store, id),
        (
            MailboxRole::Archive,
            ReadState::Unread,
            Star::Unstarred,
            vec!["Projects/2026".to_owned()]
        )
    );
    let label = store
        .labels(ACCOUNT)
        .unwrap()
        .into_iter()
        .find(|l| l.name == "Projects/2026")
        .unwrap();
    assert_eq!(label.origin, LabelOrigin::Provider, "the server's folder");
}

/// A rule may name a server folder: the message's own addresses answer it, on both stores.
fn by_folder<S: Store>(store: &S, raw: BlobId) -> Vec<Vec<String>> {
    let at = |path: &str| MailboxRef {
        account: ACCOUNT,
        path: path.to_owned(),
    };
    store
        .put_rule(&rule(
            1,
            "from the inbox",
            Filter::InFolder(at("INBOX")),
            vec![RuleAction::Star],
        ))
        .unwrap();
    store
        .put_rule(&rule(
            2,
            "from a folder",
            Filter::InFolder(at("Projects")),
            vec![RuleAction::MarkRead],
        ))
        .unwrap();
    let m = arriving(1, "ada@example.test", MailboxRole::Inbox, raw);
    let patch = store.ingest(ACCOUNT, ingest(vec![m])).unwrap();
    let ran = rules::at_arrival(store, ACCOUNT, &folders(), &arrived(&patch), at_now()).unwrap();
    names(&ran)
}

fn at_now() -> DateTime<Utc> {
    at(10)
}

#[test]
fn a_rule_about_a_folder_asks_the_messages_own_addresses() {
    let (sqlite, _dir) = sqlite();
    let raw = sqlite.blobs().put(&sqlite.connection(), b"raw").unwrap();
    let seen = by_folder(&sqlite, raw);
    assert_eq!(seen, vec![vec!["from the inbox".to_owned()]]);
    assert_eq!(by_folder(&MemoryStore::new(), BlobId::generate()), seen);
}
