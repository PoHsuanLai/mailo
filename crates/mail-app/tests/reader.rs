//! What the reader shows for a real message, including the ones that are broken.
//!
//! The sandboxed iframe in `ui.rs` had never received anything: the reader passed `None` for the
//! HTML part on every message, because the part is not a column — it lives inside the stored raw
//! bytes. So every HTML message in the world rendered as its plain-text alternative, and one
//! with no text alternative rendered as nothing at all.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_mime::{RemoteImages, SanitizePolicy};
use mail_store::{SqliteStore, Store};

#[allow(dead_code)]
#[path = "../src/query.rs"]
mod query;
#[allow(dead_code)]
#[path = "../src/view.rs"]
mod view;

#[path = "../src/reader.rs"]
mod reader;

/// The render cache is process-wide, and these tests share a process.
///
/// Anything that clears it or counts what is in it has to have it to itself — otherwise the
/// count is a race against whatever else is rendering, which is how a test that is right becomes
/// a test that fails on a busy machine and then gets deleted.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let held = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|held| held.into_inner());
    reader::forget_everything();
    held
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn policy() -> SanitizePolicy {
    SanitizePolicy {
        remote_images: RemoteImages::Blocked,
        version: SanitizePolicy::CURRENT.version,
    }
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    (store, dir)
}

/// Ingest `raw`, with `text` as the extracted plain part.
fn ingest(store: &SqliteStore, raw_bytes: &[u8], text: Option<&str>) -> Message {
    let raw = store.blobs().put(&store.connection(), raw_bytes).unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("{}@example.test", uuid::Uuid::new_v4())),
        date: at(0),
        from: Address {
            name: None,
            email: "sender@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "subject".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: text.map(str::to_owned),
            raw,
        },
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: uuid::Uuid::new_v4().to_string(),
                    },
                    key: message.key.clone(),
                    raw,
                    message: message.clone(),
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    message
}

const MULTIPART: &[u8] = b"From: sender@example.test\r\n\
Subject: subject\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
the plain alternative\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>the <b>rich</b> alternative</p>\r\n\
--b1--\r\n";

#[test]
fn an_html_message_reaches_the_sandbox_instead_of_falling_back_to_text() {
    let (store, _dir) = store();
    let message = ingest(&store, MULTIPART, Some("the plain alternative"));

    // Through `render`, which is what the shell calls: it parses the stored bytes once and
    // decides what the pane shows. Asking `html_of` separately tested a step the app no longer
    // takes on its own.
    match reader::render(&store, &message, policy()) {
        view::Reading::Html { html: rendered, .. } => {
            assert!(rendered.contains("rich"), "{rendered}");
            assert!(
                rendered.contains("<b>"),
                "formatting was stripped: {rendered}"
            );
        }
        other => panic!("an HTML message rendered as {other:?}"),
    }
}

#[test]
fn a_plain_text_message_has_no_html_part_and_renders_as_text() {
    let (store, _dir) = store();
    let raw = b"From: sender@example.test\r\nSubject: s\r\n\r\njust words\r\n";
    let message = ingest(&store, raw, Some("just words"));

    match reader::render(&store, &message, policy()) {
        view::Reading::Text(text) => assert_eq!(text, "just words"),
        other => panic!("plain text rendered as {other:?}"),
    }
}

#[test]
fn a_message_whose_bytes_do_not_parse_is_still_readable() {
    // A blank pane for mail every other client displays is worse than a rough rendering.
    let (store, _dir) = store();
    let message = ingest(
        &store,
        b"\x00\x01\x02 not a message at all",
        Some("recovered text"),
    );

    match reader::render(&store, &message, policy()) {
        view::Reading::Text(text) => assert_eq!(text, "recovered text"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_message_with_no_body_yet_is_not_mistaken_for_an_empty_one() {
    // Mid-sync. `Body::Absent` has no blob to read, and the reader must say "not downloaded"
    // rather than showing an empty message.
    let (store, _dir) = store();
    let mut message = ingest(&store, MULTIPART, None);
    message.body = Body::Absent;

    assert_eq!(
        reader::render(&store, &message, policy()),
        view::Reading::NotFetched
    );
}

#[test]
fn a_script_in_the_html_part_does_not_survive_to_the_iframe() {
    // The sanitizer has its own adversarial tests; this one proves the reader actually routes
    // through it rather than handing raw bytes to the frame.
    let (store, _dir) = store();
    let raw = b"From: sender@example.test\r\n\
Subject: s\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>hello</p><script>alert(1)</script>\r\n";
    let message = ingest(&store, raw, None);

    // The stored bytes still contain the script — nothing rewrites the message — and what
    // reaches the frame does not.
    let stored = String::from_utf8_lossy(raw).to_string();
    assert!(stored.contains("<script>"), "the fixture should carry one");

    match reader::render(&store, &message, policy()) {
        view::Reading::Html { html: rendered, .. } => {
            assert!(rendered.contains("hello"), "{rendered}");
            assert!(
                !rendered.contains("<script"),
                "a script reached the frame: {rendered}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// Inline images, through the real store and the real sanitizer.
mod inline_images {
    use super::*;

    const WITH_IMAGE: &[u8] = b"From: sender@example.test\r\n\
Subject: s\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>look</p><img src=\"cid:logo@example\">\r\n\
--b1\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@example>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--b1--\r\n";

    #[test]
    fn an_inline_image_is_resolved_rather_than_left_broken() {
        // `cid:` survives sanitizing on purpose, and nothing resolved it, so every inline image
        // in every HTML mail rendered as a broken image icon.
        let (store, _dir) = store();
        let message = ingest(&store, WITH_IMAGE, None);

        match reader::render(&store, &message, policy()) {
            view::Reading::Html { html, .. } => {
                assert!(html.contains("look"), "{html}");
                assert!(
                    html.contains("data:image/png;base64,"),
                    "the inline image was not resolved: {html}"
                );
                assert!(!html.contains("cid:"), "{html}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn resolution_happens_after_sanitizing_not_before() {
        // If the order were reversed, the sanitizer would be judging a data: URI this code
        // produced instead of the cid: the sender wrote — and anything it strips from the
        // document would be stripped from our substitution rather than from the message.
        let raw = b"From: sender@example.test\r\n\
Subject: s\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<img src=\"cid:logo@example\"><script>alert(1)</script>\r\n\
--b1\r\n\
Content-Type: image/png\r\n\
Content-ID: <logo@example>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--b1--\r\n";
        let (store, _dir) = store();
        let message = ingest(&store, raw, None);

        match reader::render(&store, &message, policy()) {
            view::Reading::Html { html, .. } => {
                assert!(!html.contains("<script"), "script survived: {html}");
                assert!(html.contains("data:image/png;base64,"), "{html}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_plain_text_message_is_untouched_by_any_of_this() {
        let (store, _dir) = store();
        let raw = b"From: sender@example.test\r\nSubject: s\r\n\r\njust words\r\n";
        let message = ingest(&store, raw, Some("just words"));
        assert_eq!(
            reader::render(&store, &message, policy()),
            view::Reading::Text("just words".to_owned())
        );
    }

    #[test]
    fn a_message_with_no_body_still_says_so() {
        let (store, _dir) = store();
        let mut message = ingest(&store, WITH_IMAGE, None);
        message.body = Body::Absent;
        assert_eq!(
            reader::render(&store, &message, policy()),
            view::Reading::NotFetched
        );
    }
}

/// Answering twice with the same answer — `plan.md` phase 8d.
///
/// `render` is a function of the stored bytes and the sanitize policy, and nothing else. The
/// bytes are named by a `BlobId`, and `BlobStore::put` is content-addressed — the same bytes are
/// the same blob — so the key is the whole of the input and a cached answer cannot quietly stop
/// being true. There is no invalidation to get wrong, which is the functional design paying for
/// itself rather than a lucky property of this one function.
///
/// The tests below are about the two ways a cache of this shape can still be wrong: giving back
/// something that is not what a fresh render would produce, and confusing two keys.
mod rendering_twice {
    use super::*;

    /// Exclusive use of the cache for the duration of a test.
    fn fresh() -> std::sync::MutexGuard<'static, ()> {
        alone()
    }

    fn allowing() -> SanitizePolicy {
        SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            version: SanitizePolicy::CURRENT.version,
        }
    }

    const REMOTE: &[u8] = b"From: ada@example.test\r\n\
        Subject: pictures\r\n\
        MIME-Version: 1.0\r\n\
        Content-Type: text/html; charset=utf-8\r\n\r\n\
        <p>hello</p><img src=\"https://tracker.test/pixel.gif\">\r\n";

    #[test]
    fn the_second_answer_is_the_first_answer() {
        let _alone = fresh();
        let (store, _dir) = store();
        let message = ingest(&store, REMOTE, Some("hello"));

        let once = reader::render(&store, &message, policy());
        let twice = reader::render(&store, &message, policy());
        assert_eq!(once, twice, "a cached render differed from a fresh one");
    }

    #[test]
    fn allowing_remote_images_is_a_different_question() {
        // The failure this prevents is a privacy failure, not a display one. If the policy were
        // not part of the key, a message rendered once with images blocked would keep its
        // blocked form after the user opted in — or, in the other direction and far worse, a
        // message rendered with images allowed would serve that markup to a conversation whose
        // owner never opted in, and the sender would get a read receipt nobody granted.
        let _alone = fresh();
        let (store, _dir) = store();
        let message = ingest(&store, REMOTE, Some("hello"));

        let blocked = reader::render(&store, &message, policy());
        let allowed = reader::render(&store, &message, allowing());

        let view::Reading::Html { html: blocked, .. } = blocked else {
            panic!("html should render as html");
        };
        let view::Reading::Html { html: allowed, .. } = allowed else {
            panic!("html should render as html");
        };
        assert!(
            !blocked.contains("tracker.test"),
            "a remote image survived blocking: {blocked}"
        );
        assert!(
            allowed.contains("tracker.test"),
            "opting in was answered from the blocked cache: {allowed}"
        );
    }

    #[test]
    fn the_order_of_the_two_questions_does_not_matter() {
        // The same pair the other way round, because a cache that is keyed correctly is keyed
        // correctly in both directions and one that is not usually fails in only one.
        let _alone = fresh();
        let (store, _dir) = store();
        let message = ingest(&store, REMOTE, Some("hello"));

        let allowed = reader::render(&store, &message, allowing());
        let blocked = reader::render(&store, &message, policy());

        let view::Reading::Html { html: allowed, .. } = allowed else {
            panic!("html should render as html");
        };
        let view::Reading::Html { html: blocked, .. } = blocked else {
            panic!("html should render as html");
        };
        assert!(allowed.contains("tracker.test"), "{allowed}");
        assert!(
            !blocked.contains("tracker.test"),
            "blocking was answered from the allowed cache, \
             which is a read receipt nobody granted: {blocked}"
        );
    }

    #[test]
    fn two_messages_with_the_same_bytes_share_one_answer() {
        // Not a coincidence to be defended against: the blob store deduplicates by hash, so the
        // same forwarded newsletter arriving twice *is* one blob, and one rendering of it is the
        // right answer for both.
        let _alone = fresh();
        let (store, _dir) = store();
        let one = ingest(&store, REMOTE, Some("hello"));
        let two = ingest(&store, REMOTE, Some("hello"));
        assert_ne!(one.id, two.id, "two messages");

        assert_eq!(
            reader::render(&store, &one, policy()),
            reader::render(&store, &two, policy())
        );
    }

    #[test]
    fn a_message_whose_body_has_not_arrived_is_not_cached_as_one_that_has() {
        // `Body::Absent` has no blob and so no key. The danger would be caching it under some
        // stand-in and then serving "not downloaded yet" after the body landed.
        let _alone = fresh();
        let (store, _dir) = store();
        let mut message = ingest(&store, REMOTE, Some("hello"));
        let real = message.body.clone();
        message.body = Body::Absent;

        assert_eq!(
            reader::render(&store, &message, policy()),
            view::Reading::NotFetched
        );

        message.body = real;
        assert!(
            matches!(
                reader::render(&store, &message, policy()),
                view::Reading::Html { .. }
            ),
            "the body arrived and the reader still said it had not"
        );
    }
}

/// Rendering a conversation before it is opened — `plan.md` phase 8e.
///
/// The one piece of phase 8 that leaves the render thread and does not need a way back onto it.
/// Everything else — a query on a blocking task, a badge count in a resource — has to deliver an
/// answer into a signal, and F140 says nothing will notice. This writes into a `Mutex` instead,
/// so an ordinary thread can fill the cache and whatever draws next simply finds it full.
mod warming_it_before_it_is_opened {
    use super::*;

    #[test]
    fn a_warmed_conversation_costs_a_lookup() {
        let _alone = alone();
        let (store, _dir) = store();
        let messages: Vec<Message> = (0..5)
            .map(|n| {
                ingest(
                    &store,
                    format!(
                        "From: ada@example.test\r\n\
                         Subject: number {n}\r\n\
                         MIME-Version: 1.0\r\n\
                         Content-Type: text/html; charset=utf-8\r\n\r\n\
                         <p>message {n}</p>\r\n"
                    )
                    .as_bytes(),
                    Some(&format!("message {n}")),
                )
            })
            .collect();

        assert_eq!(reader::held(), 0);
        let warmed = reader::prewarm(&store, &messages, policy());
        assert_eq!(warmed, 5, "every message had a body to render");
        assert_eq!(reader::held(), 5);

        // Opening it now adds nothing, because there is nothing left to do.
        for message in &messages {
            let _ = reader::render(&store, message, policy());
        }
        assert_eq!(
            reader::held(),
            5,
            "rendering a warmed conversation did the work again"
        );
    }

    #[test]
    fn warming_gives_the_same_answer_as_opening_would_have() {
        // The property that matters more than the speed: a warmed answer is the answer, not an
        // approximation of it computed under different conditions.
        let _alone = alone();
        let (store, _dir) = store();
        let message = ingest(
            &store,
            b"From: ada@example.test\r\n\
              Subject: pictures\r\n\
              MIME-Version: 1.0\r\n\
              Content-Type: text/html; charset=utf-8\r\n\r\n\
              <p>hi</p><img src=\"https://tracker.test/p.gif\">\r\n",
            Some("hi"),
        );

        let cold = {
            reader::forget_everything();
            reader::render(&store, &message, policy())
        };
        reader::forget_everything();
        reader::prewarm(&store, std::slice::from_ref(&message), policy());
        let warm = reader::render(&store, &message, policy());
        assert_eq!(cold, warm);
    }

    #[test]
    fn a_message_whose_body_has_not_arrived_is_not_warmed() {
        // Nothing to render and nothing to cache; counting it would make the return value a lie
        // about how much work was done.
        let _alone = alone();
        let (store, _dir) = store();
        let mut message = ingest(
            &store,
            b"From: a@b.test\r\nSubject: x\r\n\r\nbody\r\n",
            Some("x"),
        );
        message.body = Body::Absent;

        assert_eq!(reader::prewarm(&store, &[message], policy()), 0);
        assert_eq!(reader::held(), 0);
    }

    #[test]
    fn it_runs_on_an_ordinary_thread() {
        // The whole point, asserted rather than assumed: no runtime, no waker, no signal. If
        // this ever needs one, it has stopped being the part of phase 8 that F140 does not block.
        let _alone = alone();
        let dir = tempfile::tempdir().unwrap();
        let store =
            std::sync::Arc::new(SqliteStore::open(dir.path().join("m.db"), dir.path()).unwrap());
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'me@example.test', '{}', datetime('now'))",
                [ACCOUNT.to_string()],
            )
            .unwrap();
        let message = ingest(
            &store,
            b"From: a@b.test\r\nSubject: x\r\n\r\nbody\r\n",
            Some("x"),
        );

        let warming = store.clone();
        let done = std::thread::spawn(move || reader::prewarm(&warming, &[message], policy()))
            .join()
            .expect("the warming thread did not panic");
        assert_eq!(done, 1);
        assert_eq!(reader::held(), 1);
    }
}
