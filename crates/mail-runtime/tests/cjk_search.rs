//! Searching mail that is not in English.
//!
//! See FINDINGS F125. Full-text search cannot find a *part* of a Chinese phrase, because
//! `unicode61` makes an unbroken run of ideographs one token; the field clauses can, because
//! they are `LIKE`. Both halves are asserted here so a change to either is noticed.
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.edu', '{}', datetime('now'))",
            [acct_account().to_string()],
        )
        .unwrap();
    (store, dir)
}

fn found(store: &SqliteStore, needle: &str) -> usize {
    store
        .threads(
            &Query {
                filter: Filter::Text(TextMatch::Contains(needle.to_owned())),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 50,
                },
            },
            now(),
        )
        .unwrap()
        .items
        .len()
}

/// Ingest one Chinese message and return the store it is in.
fn with_chinese_mail() -> (SqliteStore, tempfile::TempDir) {
    let (store, dir) = store();
    let raw = "From: noreply@example.edu\r\n\
               To: me@example.edu\r\n\
               Subject: =?UTF-8?B?44CQ6YeN6KaB44CR5qCh5ZyS6YO15Lu25L+h566x57O757Wx57at6K23?=\r\n\
               Message-ID: <cjk@example.edu>\r\n\
               MIME-Version: 1.0\r\n\
               Content-Type: text/plain; charset=utf-8\r\n\
               Content-Transfer-Encoding: 8bit\r\n\
               \r\n\
               本週六凌晨兩點至六點暫停服務，造成不便敬請見諒。\r\n";
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
                uidl: "u1".to_owned(),
            },
            raw: raw.as_bytes().to_vec(),
        }],
        false,
        now(),
    )
    .unwrap();

    (store, dir)
}

fn subject_hits(store: &SqliteStore, needle: &str) -> usize {
    store
        .threads(
            &Query {
                filter: Filter::Subject(TextMatch::Contains(needle.to_owned())),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            now(),
        )
        .unwrap()
        .items
        .len()
}

/// Which clause a row asks through.
#[derive(Clone, Copy)]
enum By {
    /// `subject:`, which is `LIKE '%needle%'` on the column.
    Subject,
    /// Full text.
    Text,
}

/// `(step, clause, needle, threads found)`.
const CASES: &[(&str, By, &str, usize)] = &[
    // `LIKE '%needle%'` on the column, so substring matching works and Chinese is findable
    // through `subject:`, `from:` and `to:`.
    ("subject: part of the phrase", By::Subject, "校園", 1),
    ("subject: its end", By::Subject, "維護", 1),
    ("subject: a longer part", By::Subject, "系統維護", 1),
    // In the body rather than the subject, so not by this clause, which is correct.
    ("subject: a body word", By::Subject, "服務", 0),
    // FINDINGS F125, and the migration that answered it. `unicode61` makes one token of an
    // unbroken run of ideographs, so the index no longer reads the message columns: it reads a
    // column Rust fills with overlapping bigrams, and the needle is bigrammed the same way.
    // These rows used to assert the opposite, and were written to fail the day this changed.
    (
        "text: the whole run still matches",
        By::Text,
        "校園郵件信箱系統維護",
        1,
    ),
    ("text: part", By::Text, "校園", 1),
    ("text: part", By::Text, "郵件", 1),
    ("text: part", By::Text, "維護", 1),
    ("text: part", By::Text, "系統維護", 1),
    ("text: part", By::Text, "信箱系統", 1),
    (
        "text: in the body rather than the subject",
        By::Text,
        "暫停服務",
        1,
    ),
    ("text: the body's start", By::Text, "本週六", 1),
    // The other half, and the one bigrams could get wrong: `園校` is `校園` backwards, and
    // `件維` takes one character from each end of the subject. Neither is a bigram of it.
    ("text: not there", By::Text, "園校", 0),
    ("text: not there", By::Text, "件維", 0),
    ("text: not there", By::Text, "維郵件", 0),
    ("text: not there", By::Text, "臺北", 0),
    // The tokeniser is right for languages that put spaces between words, which is why it was
    // chosen. Whatever fixes CJK must not cost this.
    (
        "text: english is unaffected, the sender is still a word",
        By::Text,
        "noreply",
        1,
    ),
];

#[test]
fn chinese_mail_is_listed_and_found_by_its_parts() {
    let (store, _dir) = with_chinese_mail();

    // RFC 2047. Stored as the characters, not as `=?UTF-8?B?…?=`, or every Chinese subject in
    // the list would be base64.
    let listed = store
        .threads(
            &Query {
                filter: Filter::All,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            now(),
        )
        .unwrap();
    assert_eq!(
        listed.items[0].subject, "【重要】校園郵件信箱系統維護",
        "the encoded-word subject is decoded on the way in"
    );

    for &(step, by, needle, want) in CASES {
        let got = match by {
            By::Subject => subject_hits(&store, needle),
            By::Text => found(&store, needle),
        };
        assert_eq!(got, want, "{step}: {needle}");
    }
}
