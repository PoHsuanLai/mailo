//! A JMAP account end to end, against the fake in `jmap_fake`: first sync, incremental sync by
//! `Email/changes`, a local flag change drained as `Email/set`, a send through
//! `EmailSubmission`, and a push waking the watch.
//!
//! Real servers are not reachable from a test. What this proves is that the client does what
//! RFC 8620 and RFC 8621 describe, against a server written from the same pages.

mod jmap_fake;

use chrono::{DateTime, TimeZone, Utc};
use jmap_fake::{Fake, PASSWORD, USER};
use mail_domain::*;
use mail_runtime::{JmapEngine, MapSecrets, Secrets, SyncReport, Woke};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_790_000_000, 0).unwrap()
}

fn raw(n: u32, subject: &str, body: &str) -> Vec<u8> {
    format!(
        "From: Ada <ada@example.test>\r\nTo: {USER}\r\nSubject: {subject}\r\n\
         Message-ID: <m{n}@example.test>\r\nDate: Mon, 01 Sep 2026 09:0{n}:00 +0000\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    )
    .into_bytes()
}

struct Setup {
    fake: Fake,
    store: Arc<SqliteStore>,
    engine: JmapEngine,
    _dir: tempfile::TempDir,
}

async fn setup() -> Setup {
    let fake = jmap_fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let preset = presets::jmap(USER, &fake.session_url(), HttpAuth::Basic);
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                ACCOUNT.to_string(),
                USER.to_owned(),
                serde_json::to_string(&preset.plan).unwrap(),
                now().to_rfc3339(),
            ],
        )
        .unwrap();
    let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
    for purpose in [
        SecretPurpose::IncomingPassword,
        SecretPurpose::OutgoingPassword,
    ] {
        secrets
            .put(
                &SecretKey {
                    account: ACCOUNT,
                    purpose,
                },
                &Credential::Password(PASSWORD.to_owned()),
            )
            .unwrap();
    }
    let engine = JmapEngine::new(ACCOUNT, preset.plan, store.clone(), secrets).unwrap();
    Setup {
        fake,
        store,
        engine,
        _dir: dir,
    }
}

async fn pass(setup: &mut Setup) -> SyncReport {
    let (_tx, mut cancel) = watch::channel(false);
    let report = setup.engine.pass(&mut cancel, now()).await.unwrap();
    assert!(report.needs_attention.is_empty(), "{report:?}");
    report
}

/// The seeded message `n`, as the store holds it.
fn held(store: &SqliteStore, n: u32) -> Option<Message> {
    by_message_id(store, &format!("m{n}@example.test"))
}

/// The message with this `Message-ID`, as the store holds it.
fn by_message_id(store: &SqliteStore, rfc_id: &str) -> Option<Message> {
    let id: Option<String> = store
        .connection()
        .query_row(
            "SELECT id FROM messages WHERE rfc_message_id = ?1",
            [rfc_id],
            |r| r.get(0),
        )
        .ok();
    id.map(|id| {
        store
            .message(MessageId::from_uuid(id.parse().unwrap()))
            .unwrap()
    })
}

fn label_names(store: &SqliteStore, message: &Message) -> Vec<String> {
    message
        .labels
        .iter()
        .map(|l| {
            store
                .connection()
                .query_row(
                    "SELECT name FROM labels WHERE id = ?1",
                    [l.to_string()],
                    |r| r.get(0),
                )
                .unwrap()
        })
        .collect()
}

/// Seed: two in the inbox (one read and starred), one under the label Work, one draft.
fn seed(fake: &Fake) -> Vec<String> {
    let mut state = fake.state.lock().unwrap();
    vec![
        state.deliver(
            &raw(1, "Hello", "about pelicans"),
            &["mbI"],
            &[],
            "2026-09-01T09:01:00Z",
        ),
        state.deliver(
            &raw(2, "Lunch", "about sandwiches"),
            &["mbI"],
            &["$seen", "$flagged"],
            "2026-09-01T09:02:00Z",
        ),
        state.deliver(
            &raw(3, "Report", "about budgets"),
            &["mbW"],
            &["$seen"],
            "2026-09-01T09:03:00Z",
        ),
        state.deliver(
            &raw(4, "Unfinished", "draft"),
            &["mbD"],
            &["$draft"],
            "2026-09-01T09:04:00Z",
        ),
    ]
}

#[tokio::test]
async fn a_first_sync_fetches_headers_then_bodies_and_files_by_mailbox() {
    let mut s = setup().await;
    seed(&s.fake);
    let report = pass(&mut s).await;
    // Three followed emails; the draft is not synced, as an IMAP Drafts folder is not.
    assert_eq!((report.headers_fetched, report.bodies_fetched), (3, 3));
    assert_eq!(report.arrived.len(), 3);
    assert!(held(&s.store, 4).is_none());

    let hello = held(&s.store, 1).unwrap();
    assert_eq!(hello.mailbox, MailboxRole::Inbox);
    assert_eq!(
        (hello.read, hello.star),
        (ReadState::Unread, Star::Unstarred)
    );
    // The body is the message's own text, from the downloaded RFC 5322 bytes.
    assert!(matches!(&hello.body, Body::Present { text: Some(t), .. } if t.contains("pelicans")));
    let lunch = held(&s.store, 2).unwrap();
    assert_eq!((lunch.read, lunch.star), (ReadState::Read, Star::Starred));
    let report_ = held(&s.store, 3).unwrap();
    assert_eq!(report_.mailbox, MailboxRole::Archive);
    assert_eq!(label_names(&s.store, &report_), vec!["Work".to_owned()]);

    // The mailboxes became folders, the roles capabilities, and push was noticed.
    let folders = s.store.folders(ACCOUNT).unwrap();
    assert!(folders.iter().any(|f| f.path == "Work"));
    assert!(s.engine.pushes());
    // Paged listing (the fake pages by two) and chunked gets (two per get) both happened.
    assert!(s.fake.calls("Email/query").len() >= 2);
    assert!(
        s.fake
            .calls("Email/get")
            .iter()
            .all(|a| a["ids"].as_array().is_none_or(|ids| ids.len() <= 2))
    );
    assert_eq!(
        s.store.cursor(&s.engine.mailbox()).unwrap(),
        Some(SyncCursor::Jmap {
            email_state: s.fake.state.lock().unwrap().email_state(),
            mailbox_state: "m1".to_owned()
        })
    );
}

#[tokio::test]
async fn a_later_sync_asks_what_changed_and_lists_nothing() {
    let mut s = setup().await;
    let ids = seed(&s.fake);
    pass(&mut s).await;
    let listings_before = listings(&s.fake);

    {
        let mut state = s.fake.state.lock().unwrap();
        // Read on a phone, archived elsewhere, deleted elsewhere, and a new arrival.
        state.touch(&ids[0], |e| e.keywords.push("$seen".to_owned()));
        state.touch(&ids[1], |e| e.mailboxes = vec!["mbA".to_owned()]);
        state.destroy(&ids[2]);
        state.deliver(
            &raw(5, "New", "about owls"),
            &["mbI"],
            &[],
            "2026-09-02T09:00:00Z",
        );
    }
    let report = pass(&mut s).await;
    assert_eq!((report.headers_fetched, report.bodies_fetched), (1, 1));
    assert_eq!(held(&s.store, 1).unwrap().read, ReadState::Read);
    assert_eq!(held(&s.store, 2).unwrap().mailbox, MailboxRole::Archive);
    assert!(held(&s.store, 3).is_none());
    assert_eq!(held(&s.store, 5).unwrap().mailbox, MailboxRole::Inbox);
    // `Email/changes` answered it; the only query was the count, which lists no ids.
    assert!(!s.fake.calls("Email/changes").is_empty());
    assert_eq!(listings(&s.fake), listings_before);
}

/// Queries that listed ids, as opposed to asking for the count alone.
fn listings(fake: &Fake) -> usize {
    fake.calls("Email/query")
        .iter()
        .filter(|a| a["limit"].as_u64() != Some(0))
        .count()
}

#[tokio::test]
async fn a_flag_changed_here_reaches_the_server_as_a_patch() {
    let mut s = setup().await;
    let ids = seed(&s.fake);
    pass(&mut s).await;
    let hello = held(&s.store, 1).unwrap();
    s.store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageStar(hello.id, Star::Starred)],
            },
        )
        .unwrap();
    s.store
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![hello.id],
                read: None,
                star: Some(Star::Starred),
            },
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageStar(hello.id, Star::Unstarred)],
            },
            now(),
        )
        .unwrap();
    let report = pass(&mut s).await;
    assert_eq!(report.outbox_settled, 1);
    let sets = s.fake.calls("Email/set");
    let mut want = serde_json::Map::new();
    want.insert(
        ids[0].clone(),
        serde_json::json!({ "keywords/$flagged": true }),
    );
    assert_eq!(
        sets.last().unwrap()["update"],
        serde_json::Value::Object(want)
    );
    let keywords = s
        .fake
        .state
        .lock()
        .unwrap()
        .email(&ids[0])
        .unwrap()
        .keywords
        .clone();
    assert!(keywords.contains(&"$flagged".to_owned()));
    // And the next pass leaves it starred rather than undoing it.
    pass(&mut s).await;
    assert_eq!(held(&s.store, 1).unwrap().star, Star::Starred);
}

#[tokio::test]
async fn a_send_is_imported_submitted_with_every_recipient_and_filed_in_sent() {
    let mut s = setup().await;
    pass(&mut s).await;
    // Frozen bytes carry no Bcc header; the envelope carries the blind recipient.
    let frozen = format!(
        "From: {USER}\r\nTo: ada@example.test\r\nSubject: Minutes\r\n\
         Message-ID: <sent1@example.test>\r\nDate: Mon, 01 Sep 2026 10:00:00 +0000\r\n\r\nhere\r\n"
    );
    let blob = s
        .store
        .blobs()
        .put(&s.store.connection(), frozen.as_bytes())
        .unwrap();
    s.store
        .enqueue(
            ACCOUNT,
            RemoteIntent::Send {
                draft: DraftId::generate(),
                raw: blob,
                mail_from: USER.to_owned(),
                rcpt_to: vec![
                    "ada@example.test".to_owned(),
                    "blind@example.test".to_owned(),
                ],
            },
            &Patch {
                id: ChangeId::generate(),
                changes: vec![],
            },
            now(),
        )
        .unwrap();
    let report = pass(&mut s).await;
    assert_eq!(report.submitted, 1);
    // What went up is the frozen message — sealed, where compose sealed it — with only its
    // `Date` set to when it left, exactly as the SMTP path sends it.
    let uploaded: Vec<Vec<u8>> = s
        .fake
        .state
        .lock()
        .unwrap()
        .blobs
        .iter()
        .filter(|(id, _)| id.starts_with('U'))
        .map(|(_, raw)| raw.clone())
        .collect();
    assert_eq!(uploaded, vec![mail_mime::restamp(frozen.as_bytes(), now())]);
    let (submitted, email) = {
        let state = s.fake.state.lock().unwrap();
        let submitted = state.submitted.clone();
        let email = state.email(&submitted[0].email).cloned().unwrap();
        (submitted, email)
    };
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0].identity, "I1");
    assert_eq!(submitted[0].mail_from, USER);
    assert_eq!(
        submitted[0].rcpt_to,
        vec![
            "ada@example.test".to_owned(),
            "blind@example.test".to_owned()
        ]
    );
    // Moved from Drafts to Sent on success, and no longer a draft.
    assert_eq!(email.mailboxes, vec!["mbS".to_owned()]);
    assert!(!email.keywords.contains(&"$draft".to_owned()));
    // The next pass brings the sent copy in, filed as Sent.
    pass(&mut s).await;
    assert_eq!(
        by_message_id(&s.store, "sent1@example.test").map(|m| m.mailbox),
        Some(MailboxRole::Sent)
    );
}

#[tokio::test]
async fn a_push_with_a_new_state_wakes_the_watch() {
    let mut s = setup().await;
    seed(&s.fake);
    pass(&mut s).await;
    let fake = s.fake.clone();
    // The stream opens with the current state, which is not news; then mail arrives.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let state = {
            let mut state = fake.state.lock().unwrap();
            state.deliver(
                &raw(6, "Pushed", "hello"),
                &["mbI"],
                &[],
                "2026-09-03T09:00:00Z",
            );
            state.email_state()
        };
        fake.push_state(&state);
    });
    let (_tx, mut cancel) = watch::channel(false);
    let woke = tokio::time::timeout(
        Duration::from_secs(10),
        s.engine.wait(&mut cancel, Duration::from_secs(3600)),
    )
    .await
    .expect("the push should wake the wait long before the poll interval")
    .unwrap();
    assert_eq!(woke, Woke::Mail);
    let report = pass(&mut s).await;
    assert_eq!(report.headers_fetched, 1);
    assert!(held(&s.store, 6).is_some());
}

#[tokio::test]
async fn a_refused_password_stops_the_pass_and_says_so() {
    let mut s = setup().await;
    let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
    secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password("wrong".to_owned()),
        )
        .unwrap();
    let plan = presets::jmap(USER, &s.fake.session_url(), HttpAuth::Basic).plan;
    s.engine = JmapEngine::new(ACCOUNT, plan, s.store.clone(), secrets).unwrap();
    let (_tx, mut cancel) = watch::channel(false);
    let report = s.engine.pass(&mut cancel, now()).await.unwrap();
    assert!(report.needs_reauth, "{report:?}");
}
