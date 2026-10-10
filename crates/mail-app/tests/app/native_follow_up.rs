//! Remind me if no reply, driven the way its user drives it in the real window on Blitz
//! (`ds_harness::Harness`): the composer's Remind row sets it, the message's copy comes back from
//! the Sent folder as a sync brings it, the Waiting place lists the conversation, and when its
//! time comes with nobody answering the conversation is back on top of the inbox saying
//! "No reply yet", and a notification says so once.
//!
//! The harness runs on quire's virtual clock and the window is handed a wall clock that follows
//! it, so a day passes in one `advance`. Everything is over a store in a `TempDir`, and the window
//! is handed no directories and a recorder for notifications: nothing here touches the real
//! store, config or desktop.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};

use crate::drive;
use drive::{Drive, PRIMARY};
use mail_app::ui::native::WallClock;
use mail_core::notify::{Notification, Notifier, Opens};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, Destination, absorb, absorb_into};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The seeded inbox, newest first: (sender, subject).
const INBOX: [(&str, &str); 3] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
];

/// What the test sends.
const SUBJECT: &str = "Checking in on the proposal";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A store in `dir` with one account that sends, its identity and capabilities, and [`INBOX`].
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            "me@example.test",
            None,
        );
        let caps = AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::DropInbox,
            folders: FolderRoles::default(),
            condstore: Condstore::Supported,
            move_ext: MoveExt::Supported,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Yes,
            connections: ConnectionBudget::default(),
            observed_at: chrono::Utc::now(),
        };
        mail_store::testing::seed_caps(&store, acct_account(), &caps, chrono::Utc::now()).unwrap();
    }
    let now = chrono::Utc::now();
    for (n, (from, subject)) in INBOX.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <inbox{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
        );
        absorb(
            &store,
            acct_account(),
            MailboxRef {
                account: acct_account(),
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("inbox{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    Arc::new(store)
}

/// What the window was asked to say on the desktop.
#[derive(Default)]
struct Recorder(Mutex<Vec<Notification>>);

impl Notifier for Recorder {
    fn show(&self, notification: &Notification) {
        self.0.lock().unwrap().push(notification.clone());
    }
}

/// The window on the virtual clock, over a freshly seeded store, first frame drawn. Its wall
/// clock starts at the real one's now and moves with the harness's. The `TempDir` must outlive
/// the rest.
fn open() -> (
    Harness,
    tempfile::TempDir,
    Arc<SqliteStore>,
    Arc<Recorder>,
    WallClock,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let recorder = Arc::new(Recorder::default());
    let base = chrono::Utc::now();
    // Read first on the harness's thread, where `ds::base::time::clock::now` is the virtual clock.
    let origin: Arc<OnceLock<Instant>> = Arc::new(OnceLock::new());
    let clock = mail_app::ui::native::WallClock::new(move || {
        let since = ds::base::time::clock::since(*origin.get_or_init(ds::base::time::clock::now));
        base + chrono::TimeDelta::from_std(since).unwrap()
    });
    // No test may open the system's print dialog.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(clock.clone())
    .with(mail_app::ui::native::Notices(recorder.clone()));
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    (harness, dir, store, recorder, clock)
}

/// Poll until `done` holds, a little virtual time at a step and for at most ten seconds of the
/// machine's: the list is read on a blocking thread, which runs on real time however fast the
/// virtual clock moves.
fn until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if done(harness) {
            return;
        }
        harness.advance(ms(10));
        std::thread::yield_now();
    }
    assert!(done(harness), "{what}:\n{}", harness.html());
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(if c == ' ' { Key::Space } else { Key::Char(c) });
    }
}

/// The open menu's row that says `name`. The composer's reminder is a menu; the reader's bell
/// opens a pick list. Each row is a direct child of its list.
fn menu_row(harness: &Harness, name: &str) -> String {
    let mut candidates = Vec::new();
    for n in 1..=24 {
        candidates.push(format!(".ds-menu > .ds-menu-item:nth-child({n})"));
        candidates.push(format!(".ds-pick-list-rows > .ds-row:nth-child({n})"));
    }
    candidates
        .into_iter()
        .find(|row| harness.text_of(row).is_some_and(|text| text.contains(name)))
        .unwrap_or_else(|| panic!("no menu row says {name:?}:\n{}", harness.html()))
}

/// Press `name` once its centre is the row itself: a menu just opened can still be on its way
/// to where it is drawn, and a press there lands on whatever is underneath.
fn press_menu_row(harness: &mut Harness, name: &str) {
    let row = menu_row(harness, name);
    until(harness, "the row can be pressed", |h| {
        h.centre(&row).is_some_and(|at| h.hits(at, &row))
    });
    harness.click(centre(harness, &row));
}

/// The `n`th row of the list (1-based).
fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

/// The reminder on the conversation that holds the message sent from `draft`, if it is here.
fn follow_up_of(store: &SqliteStore, draft: DraftId) -> Option<(ThreadId, FollowUp)> {
    let thread = mail_store::testing::thread_with_rfc_prefix(store, &format!("{draft}@"))?;
    Some((thread, store.thread(thread).ok()?.summary.follow_up))
}

/// The bytes the outbox holds for `draft`: what the server was given, and what its Sent folder
/// hands back.
fn sent_bytes(store: &SqliteStore, draft: DraftId) -> Vec<u8> {
    let op = mail_store::testing::outbox_ops(store)
        .into_iter()
        .next()
        .expect("the outbox holds an operation");
    let ProtoOp::Submit { raw, draft: of, .. } = op else {
        panic!("the outbox holds no submission: {op:?}");
    };
    assert_eq!(of, draft);
    store.blobs().get(raw).unwrap()
}

#[test]
fn a_reminder_set_in_the_composer_brings_the_conversation_back_with_no_reply_yet() {
    // The window's now, which the test reads on the same thread and so on the same clock.
    let (mut harness, _dir, store, recorder, clock) = open();
    let base = clock.now();
    let inbox_before = harness.count(".list .ds-thread");
    assert_eq!(inbox_before, INBOX.len());

    // A new message: to, subject, and the Remind row's "Tomorrow morning".
    harness.key(Key::Char('c'));
    until(&mut harness, "the composer opens", |h| {
        h.count(".cpage .c-body") == 1
    });
    // The page slides in; it is pressed where it comes to rest.
    harness.advance(ms(600));
    let to = ".c-props [*|data-row=to] input";
    harness.click(centre(&harness, to));
    until(&mut harness, "To has the keyboard", |h| h.is_focused(to));
    type_text(&mut harness, "ada@example.test");
    harness.key(Key::Enter);
    until(&mut harness, "Ada is a chip", |h| {
        h.count(".c-props [*|data-row=to] .ds-chip") == 1
    });
    harness.click(centre(&harness, ".c-title input"));
    until(&mut harness, "the subject has the keyboard", |h| {
        h.is_focused(".c-title input")
    });
    type_text(&mut harness, SUBJECT);
    let remind = ".c-props [*|aria-label=\"Remind me if no reply\"]";
    harness.click(centre(&harness, remind));
    until(&mut harness, "the Remind menu opens", |h| {
        h.count(".ds-menu .ds-menu-item") > 0
    });
    press_menu_row(&mut harness, "Tomorrow morning");
    until(&mut harness, "the row says the reminder", |h| {
        h.count(".ds-menu") == 0
            && h.text_of(".c-props [*|data-row=remind]")
                .is_some_and(|text| text.contains("Tomorrow morning, if no reply"))
    });

    harness.click(centre(&harness, ".c-foot [*|aria-label=\"Send\"]"));
    until(&mut harness, "the page folds away", |h| {
        h.count(".cpage .c-body") == 0
    });
    let drafts = store.drafts(acct_account()).unwrap();
    assert_eq!(drafts.len(), 1);
    let draft = drafts[0].id;
    assert_eq!(drafts[0].state, SendState::Queued);
    // Not on any conversation yet: the message has not come back from the server.
    assert_eq!(follow_up_of(&store, draft), None);

    // The next sync fetches the Sent folder, and the copy of what was sent is in it.
    absorb_into(
        &store,
        acct_account(),
        Destination {
            mailbox: MailboxRef {
                account: acct_account(),
                path: "Sent".to_owned(),
            },
            role: MailboxRole::Sent,
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: "sent0".to_owned(),
            },
            raw: sent_bytes(&store, draft),
        }],
        false,
        chrono::Utc::now(),
    )
    .unwrap();
    // The window's next sweep puts the reminder on it, due tomorrow at nine.
    harness.advance(Duration::from_secs(6 * 60));
    let (thread, waiting) = follow_up_of(&store, draft).expect("the Sent copy is stored");
    let FollowUp::Until { at, .. } = waiting else {
        panic!("the reminder did not join its conversation: {waiting:?}");
    };
    let local = at.with_timezone(&chrono::Local);
    assert_eq!(
        (local.date_naive(), local.format("%H:%M").to_string()),
        (
            base.with_timezone(&chrono::Local).date_naive() + chrono::TimeDelta::days(1),
            "09:00".to_owned()
        ),
        "tomorrow morning"
    );

    // The Waiting place lists it; the inbox does not, since only the user has written.
    let waiting_place = "[*|data-place=\"Waiting\"]";
    harness.click(centre(&harness, waiting_place));
    until(&mut harness, "the Waiting place lists it", |h| {
        h.count(".list .ds-thread") == 1
            && h.text_of(&format!("{} .ds-thread-sub", row(1)))
                .is_some_and(|subject| subject.contains(SUBJECT))
    });
    harness.click(centre(&harness, "[*|data-place=\"Inbox\"]"));
    until(&mut harness, "the inbox is as it was", |h| {
        h.count(".list .ds-thread") == inbox_before
    });
    assert_eq!(harness.count(".no-reply"), 0);
    assert!(recorder.0.lock().unwrap().is_empty());

    // A minute before its time: still waiting.
    let before = (at - clock.now()).to_std().unwrap() - Duration::from_secs(60);
    harness.advance(before);
    assert!(
        matches!(
            follow_up_of(&store, draft),
            Some((_, FollowUp::Until { .. }))
        ),
        "it came back early"
    );
    assert_eq!(harness.count(".no-reply"), 0);

    // Its time, and nobody has answered: back on top of the inbox, saying so, and said once.
    harness.advance(Duration::from_secs(2 * 60));
    assert!(matches!(
        follow_up_of(&store, draft),
        Some((_, FollowUp::Returned { .. }))
    ));
    until(&mut harness, "the conversation is back on top", |h| {
        h.count(".list .ds-thread") == inbox_before + 1
            && h.text_of(&format!("{} .ds-thread-sub", row(1)))
                .is_some_and(|subject| subject.contains(SUBJECT))
    });
    let words = harness
        .text_of(&format!("{} .no-reply", row(1)))
        .unwrap_or_default();
    assert!(words.contains("No reply yet"), "{words:?}");
    assert_eq!(harness.count(".no-reply"), 1, "only that row says it");
    let said = recorder.0.lock().unwrap().clone();
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].summary, "No reply yet");
    assert_eq!(said[0].body, SUBJECT);
    assert_eq!(said[0].opens, Opens::Thread(thread));

    // A day on, nothing is said again.
    harness.advance(Duration::from_secs(24 * 60 * 60));
    assert_eq!(recorder.0.lock().unwrap().len(), 1, "announced once");
}

/// The reader's bell sets a reminder on the open conversation, the reader says so, and Ctrl Z
/// takes it back like any other gesture.
#[test]
fn the_reader_s_bell_sets_a_reminder_and_ctrl_z_takes_it_back() {
    let (mut harness, _dir, store, _recorder, clock) = open();
    let subject = format!("{} .ds-thread-sub", row(2));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    until(&mut harness, "the reader opens", |h| {
        h.count(".reader-head h2") == 1
    });
    let thread = |store: &SqliteStore| {
        let query = Query {
            filter: Filter::Subject(TextMatch::Contains(INBOX[1].1.to_owned())),
            sort: Sort {
                property: Property::Date,
                dir: SortDir::Desc,
            },
            page: PageReq {
                after: None,
                limit: 1,
            },
        };
        store.threads(&query, chrono::Utc::now()).unwrap().items[0].follow_up
    };
    assert_eq!(thread(&store), FollowUp::Inactive);
    assert_eq!(harness.count(".reader-head .follow-up-note"), 0);

    let bell = ".reader-head [*|aria-label=\"Remind me if no reply\"]";
    harness.click(centre(&harness, bell));
    until(&mut harness, "the menu opens", |h| {
        h.count(".ds-pick-list-rows > .ds-row") > 0
    });
    press_menu_row(&mut harness, "In 3 days");
    until(&mut harness, "the reader says it", |h| {
        h.text_of(".reader-head .follow-up-note")
            .is_some_and(|note| note.contains("if nobody replies"))
    });
    let FollowUp::Until { at, set } = thread(&store) else {
        panic!("no reminder: {:?}", thread(&store));
    };
    assert!(
        set <= clock.now() && clock.now() - set < chrono::TimeDelta::minutes(1),
        "set as it was asked"
    );
    assert_eq!(
        at.with_timezone(&chrono::Local).date_naive(),
        set.with_timezone(&chrono::Local).date_naive() + chrono::TimeDelta::days(3)
    );
    assert_eq!(
        harness.attr(bell, "aria-pressed").as_deref(),
        Some("true"),
        "the bell is pressed while a reminder stands"
    );

    harness.chord(&[PRIMARY], Key::Char('z'));
    until(&mut harness, "Ctrl Z took it back", |h| {
        h.count(".reader-head .follow-up-note") == 0
    });
    assert_eq!(thread(&store), FollowUp::Inactive);
}
