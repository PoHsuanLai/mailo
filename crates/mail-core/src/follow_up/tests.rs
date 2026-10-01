use super::*;
use mail_domain::{AccountId, Draft, SendState};
use mail_domain::{
    Address, Body, Change, ChangeId, MailboxRole, MessageId, MessageKey, Patch, ReadState, Star,
};
use std::sync::Mutex;

const ACCOUNT: AccountId = AccountId::from_uuid(uuid::Uuid::from_u128(0xa1));
const ME: &str = "me@example.test";

/// Noon on a Monday, 2026-09-28, and whole days from it.
fn day(n: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap() + TimeDelta::days(n)
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
}

fn store(dir: &std::path::Path) -> SqliteStore {
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?2, '{}', datetime('now'))",
            [ACCOUNT.to_string(), ME.to_owned()],
        )
        .unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, ?3, '\"default\"')",
            [
                mail_domain::IdentityId::generate().to_string(),
                ACCOUNT.to_string(),
                ME.to_owned(),
            ],
        )
        .unwrap();
    store
}

/// A message in thread `t`, from `from`, at `at`, filed as `role`.
fn message(n: u128, t: ThreadId, from: &str, at: DateTime<Utc>, role: MailboxRole) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        thread: t,
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at,
        from: Address {
            name: None,
            email: from.to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("subject {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@example.test")),
        read: ReadState::Read,
        star: Star::Unstarred,
        mailbox: role,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn put(store: &SqliteStore, message: Message) {
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(message))],
            },
        )
        .unwrap();
}

fn state(store: &SqliteStore, t: ThreadId) -> FollowUp {
    store.thread(t).unwrap().summary.follow_up
}

#[test]
fn the_named_times_and_a_typed_one_resolve_from_when_it_leaves() {
    // Monday noon in UTC.
    let from = day(0);
    let cases: [(&str, Result<DateTime<Utc>, ()>); 7] = [
        (
            "tomorrow",
            Ok(Utc.with_ymd_and_hms(2026, 9, 29, 9, 0, 0).unwrap()),
        ),
        (
            "three-days",
            Ok(Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap()),
        ),
        (
            "next-week",
            Ok(Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap()),
        ),
        ("+2h", Ok(from + TimeDelta::hours(2))),
        (
            "2026-10-02 17:30",
            Ok(Utc.with_ymd_and_hms(2026, 10, 2, 17, 30, 0).unwrap()),
        ),
        // Gone already, and not a time at all.
        ("2026-09-01", Err(())),
        ("", Err(())),
    ];
    for (choice, want) in cases {
        let got = due(choice, from, &Utc);
        match want {
            Ok(at) => assert_eq!(got, Ok(at), "{choice:?}"),
            Err(()) => assert!(got.is_err(), "{choice:?} should be refused: {got:?}"),
        }
    }
}

#[test]
fn a_reply_is_someone_else_writing_after_the_reminder_was_set() {
    let own = Own::new([ME, "alias@example.test"]);
    let set = day(0);
    let t = thread(1);
    let mine = message(1, t, ME, day(-1), MailboxRole::Sent);
    // (name, the conversation's messages, replied)
    let cases: [(&str, Vec<Message>, bool); 5] = [
        ("only what I sent", vec![mine.clone()], false),
        (
            "their earlier mail, which I was answering",
            vec![
                message(2, t, "ada@example.test", day(-2), MailboxRole::Inbox),
                mine.clone(),
            ],
            false,
        ),
        (
            "my own later note, from another of my addresses",
            vec![
                mine.clone(),
                message(3, t, "ALIAS@example.test", day(1), MailboxRole::Sent),
            ],
            false,
        ),
        (
            "their answer after",
            vec![
                mine.clone(),
                message(4, t, "ada@example.test", day(1), MailboxRole::Inbox),
            ],
            true,
        ),
        (
            "an answer a rule filed away still counts",
            vec![
                mine,
                message(5, t, "grace@example.test", day(2), MailboxRole::Archive),
            ],
            true,
        ),
    ];
    for (name, messages, want) in cases {
        assert_eq!(replied(&messages, &own, set), want, "{name}");
    }
}

/// Records what it was asked to show.
#[derive(Default)]
struct Recorder(Mutex<Vec<Notification>>);

impl Notifier for Recorder {
    fn show(&self, notification: &Notification) {
        self.0.lock().unwrap().push(notification.clone());
    }
}

#[test]
fn no_reply_brings_it_back_at_its_time_once_and_a_reply_clears_it_quietly() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let quiet = thread(1);
    let answered = thread(2);
    put(&store, message(1, quiet, ME, day(0), MailboxRole::Sent));
    put(&store, message(2, answered, ME, day(0), MailboxRole::Sent));
    for t in [quiet, answered] {
        crate::snooze::apply(&store, t, remind(day(3), day(0)), day(0)).unwrap();
    }
    let recorder = Recorder::default();
    let shown = || recorder.0.lock().unwrap().len();

    // Day 1: someone answers the second.
    put(
        &store,
        message(3, answered, "ada@example.test", day(1), MailboxRole::Inbox),
    );
    let swept = sweep_and_announce(&store, Some(&recorder), day(1)).unwrap();
    assert_eq!(
        swept.cleared,
        vec![answered],
        "the answer clears it at once"
    );
    assert_eq!(state(&store, answered), FollowUp::Inactive);
    assert_eq!(state(&store, quiet), remind_state(day(3), day(0)));
    assert!(swept.returned.is_empty());
    assert_eq!(shown(), 0, "clearing is quiet");

    // A second before its time: still waiting, and not in the inbox.
    let before = day(3) - TimeDelta::seconds(1);
    assert!(!sweep(&store, before).unwrap().changed());
    assert_eq!(returned(&store, None, before), Vec::new());

    // At its time, nobody has answered the first: back, announced once.
    let swept = sweep_and_announce(&store, Some(&recorder), day(3)).unwrap();
    let back = FollowUp::Returned {
        at: day(3),
        set: day(0),
    };
    assert_eq!(state(&store, quiet), back);
    assert_eq!(
        swept.returned.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![quiet]
    );
    assert_eq!(shown(), 1);
    let said = recorder.0.lock().unwrap()[0].clone();
    assert_eq!(said.summary, "No reply yet");
    assert_eq!(said.body, "subject 1");
    assert_eq!(said.opens, Opens::Thread(quiet));
    // On top of the inbox, though it holds only what the user sent.
    let top = returned(&store, None, day(3));
    assert_eq!(top.iter().map(|s| s.id).collect::<Vec<_>>(), vec![quiet]);
    let listed = on_top(top, vec![store.thread(answered).unwrap().summary]);
    assert_eq!(
        listed.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![quiet, answered]
    );
    assert_eq!(row_words(&listed[0].follow_up), Some("No reply yet"));

    // The next sweep, or the next launch's, says nothing again.
    let again = sweep_and_announce(&store, Some(&recorder), day(4)).unwrap();
    assert!(!again.changed(), "{again:?}");
    assert_eq!(shown(), 1, "announced once");

    // A late answer clears the returned one too, and it leaves the top of the inbox.
    put(
        &store,
        message(4, quiet, "grace@example.test", day(5), MailboxRole::Inbox),
    );
    let swept = sweep(&store, day(5)).unwrap();
    assert_eq!(swept.cleared, vec![quiet]);
    assert_eq!(returned(&store, None, day(5)), Vec::new());
    assert_eq!(store.follow_ups().unwrap(), Vec::new());
}

fn remind_state(at: DateTime<Utc>, set: DateTime<Utc>) -> FollowUp {
    FollowUp::Until { at, set }
}

#[test]
fn a_reminder_that_came_due_while_mailo_was_closed_comes_back_at_the_first_sweep() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let t = thread(1);
    put(&store, message(1, t, ME, day(0), MailboxRole::Sent));
    crate::snooze::apply(&store, t, remind(day(1), day(0)), day(0)).unwrap();
    assert_eq!(next_due(&store), Some(day(1)));
    // Nothing ran on day 1. The first sweep after is a week later.
    let swept = sweep(&store, day(8)).unwrap();
    assert_eq!(swept.returned.len(), 1);
    assert_eq!(
        state(&store, t),
        FollowUp::Returned {
            at: day(1),
            set: day(0)
        },
        "it keeps the time it was due"
    );
    assert_eq!(next_due(&store), None, "nothing is waiting any more");
}

#[test]
fn a_returned_conversation_leaves_the_top_when_archived_or_snoozed() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let t = thread(1);
    put(&store, message(1, t, ME, day(0), MailboxRole::Sent));
    crate::snooze::apply(&store, t, remind(day(1), day(0)), day(0)).unwrap();
    sweep(&store, day(1)).unwrap();
    assert_eq!(returned(&store, None, day(1)).len(), 1);

    crate::snooze::apply(&store, t, Op::SetSnooze(Snooze::Until(day(2))), day(1)).unwrap();
    assert_eq!(returned(&store, None, day(1)).len(), 0, "snoozed away");
    assert_eq!(returned(&store, None, day(2)).len(), 1, "and back with it");

    crate::snooze::apply(&store, t, Op::Archive, day(2)).unwrap();
    assert_eq!(returned(&store, None, day(2)).len(), 0, "archived");
    // Still waiting in the Waiting place, which lists every reminder.
    assert_eq!(waiting(&store, None, day(2)).len(), 1);
    // And not on another account's list.
    let elsewhere = Filter::Account(AccountId::from_uuid(uuid::Uuid::from_u128(0xa2)));
    assert_eq!(waiting(&store, Some(&elsewhere), day(2)).len(), 0);
}

/// A draft queued to leave, as `crate::compose` leaves it.
fn queued(store: &SqliteStore, in_reply_to: Option<MessageId>) -> Draft {
    let draft =
        crate::compose::draft_new(store, ACCOUNT, &[], "Checking in", "Any news?", day(0)).unwrap();
    let draft = Draft {
        in_reply_to,
        state: SendState::Queued,
        ..draft
    };
    crate::compose::save(store, &draft).unwrap();
    draft
}

fn raw_with_id(id: &str) -> Vec<u8> {
    format!(
        "From: {ME}\r\nTo: ada@example.test\r\nSubject: Checking in\r\n\
         Message-ID: <{id}>\r\nDate: Mon, 28 Sep 2026 12:00:00 +0000\r\n\r\nAny news?\r\n"
    )
    .into_bytes()
}

#[test]
fn a_new_message_s_reminder_joins_the_conversation_its_sent_copy_lands_in() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let draft = queued(&store, None);
    after_queue(
        &store,
        &draft,
        &raw_with_id("Out.1@Example.test"),
        Some(day(3)),
        day(0),
    )
    .unwrap();
    assert_eq!(held(&store).len(), 1);
    assert_eq!(
        next_due(&store),
        None,
        "held, not yet waiting on a conversation"
    );

    // Not found yet: nothing moves.
    assert!(!sweep(&store, day(0)).unwrap().changed());

    // The Sent folder's copy arrives, under the id it was sent with.
    let t = thread(9);
    let mut copy = message(9, t, ME, day(0), MailboxRole::Sent);
    copy.rfc_message_id = Some("out.1@example.test".to_owned());
    put(&store, copy);
    let swept = sweep(&store, day(0)).unwrap();
    assert_eq!(swept.attached, vec![t]);
    assert_eq!(state(&store, t), remind_state(day(3), day(0)));
    assert_eq!(held(&store), Vec::new(), "held no longer");

    // Sending it again without a reminder, or taking it back, lets a held one go.
    let again = queued(&store, None);
    after_queue(
        &store,
        &again,
        &raw_with_id("out.2@example.test"),
        Some(day(3)),
        day(0),
    )
    .unwrap();
    assert_eq!(held(&store).len(), 1);
    after_queue(
        &store,
        &again,
        &raw_with_id("out.2@example.test"),
        None,
        day(0),
    )
    .unwrap();
    assert_eq!(held(&store), Vec::new());
    after_queue(
        &store,
        &again,
        &raw_with_id("out.2@example.test"),
        Some(day(3)),
        day(0),
    )
    .unwrap();
    crate::compose::unsend(&store, again.id, day(0)).unwrap();
    assert_eq!(
        held(&store),
        Vec::new(),
        "Undo send takes the reminder back too"
    );
}

#[test]
fn a_reply_s_reminder_joins_its_conversation_once_the_outbox_has_sent_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let t = thread(1);
    let theirs = message(1, t, "ada@example.test", day(-1), MailboxRole::Inbox);
    put(&store, theirs.clone());
    let draft = queued(&store, Some(theirs.id));
    after_queue(
        &store,
        &draft,
        &raw_with_id("reply.1@example.test"),
        Some(day(3)),
        day(0),
    )
    .unwrap();
    assert_eq!(held(&store)[0].thread, Some(t));

    assert!(
        !sweep(&store, day(0)).unwrap().changed(),
        "still in the outbox, where Undo can take it back"
    );
    store
        .set_send_state(
            draft.id,
            &SendState::Sent {
                at: day(0),
                message: None,
            },
            day(0),
        )
        .unwrap();
    let swept = sweep(&store, day(0)).unwrap();
    assert_eq!(swept.attached, vec![t]);
    assert_eq!(state(&store, t), remind_state(day(3), day(0)));
    // Their earlier mail is what the user answered: it is not a reply to the reminder.
    assert!(swept.cleared.is_empty());
}

#[test]
fn a_held_reminder_whose_message_never_comes_back_is_let_go_a_week_after_its_time() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let draft = queued(&store, None);
    after_queue(
        &store,
        &draft,
        &raw_with_id("lost@example.test"),
        Some(day(1)),
        day(0),
    )
    .unwrap();
    sweep(&store, day(7)).unwrap();
    assert_eq!(held(&store).len(), 1, "six days late, still looked for");
    sweep(&store, day(8)).unwrap();
    assert_eq!(held(&store), Vec::new());
}
