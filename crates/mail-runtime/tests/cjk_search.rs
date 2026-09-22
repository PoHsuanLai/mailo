//! Searching mail that is not in English, which is half of this user's.
//!
//! See FINDINGS F125. Full-text search cannot find a *part* of a Chinese phrase, because
//! `unicode61` makes an unbroken run of ideographs one token; the field clauses can, because
//! they are `LIKE`. Both halves are asserted here so a change to either is noticed.
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

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
             VALUES (?1, 'me@ntu.edu.tw', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
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
    let raw = "From: ccnoreply@ntu.edu.tw\r\n\
               To: me@ntu.edu.tw\r\n\
               Subject: =?UTF-8?B?44CQ6YeN6KaB44CR6Ie65aSn6KiI5Lit5L+h566x57O757Wx57at6K23?=\r\n\
               Message-ID: <cjk@ntu.edu.tw>\r\n\
               MIME-Version: 1.0\r\n\
               Content-Type: text/plain; charset=utf-8\r\n\
               Content-Transfer-Encoding: 8bit\r\n\
               \r\n\
               本週六凌晨兩點至六點暫停服務，造成不便敬請見諒。\r\n";
    absorb(
        &store,
        ACCOUNT,
        MailboxRef {
            account: ACCOUNT,
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

#[test]
fn an_encoded_word_subject_is_decoded_on_the_way_in() {
    // RFC 2047. Stored as the characters, not as `=?UTF-8?B?…?=`, or every Chinese subject in
    // the list would be base64.
    let (store, _dir) = with_chinese_mail();
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
    assert_eq!(listed.items[0].subject, "【重要】臺大計中信箱系統維護");
}

#[test]
fn a_field_clause_finds_part_of_a_chinese_phrase() {
    // `LIKE '%needle%'` on the column, so substring matching works and Chinese is findable
    // through `subject:`, `from:` and `to:` — which is the whole of what works today.
    let (store, _dir) = with_chinese_mail();
    assert_eq!(subject_hits(&store, "臺大"), 1);
    assert_eq!(subject_hits(&store, "維護"), 1);
    assert_eq!(subject_hits(&store, "系統維護"), 1);
    // In the body rather than the subject, so not by this clause — which is correct.
    assert_eq!(subject_hits(&store, "服務"), 0);
}

#[test]
fn full_text_cannot_find_part_of_a_chinese_phrase() {
    // FINDINGS F125, asserted as it is rather than as it should be. `unicode61` classifies
    // ideographs as token characters, so an unbroken run of them is ONE token: the whole
    // subject is a single word, and only typing all of it finds anything.
    //
    // This test says what the search box does today. When the index learns to segment CJK it
    // will fail, which is the point: the day that changes is the day this file should.
    let (store, _dir) = with_chinese_mail();
    assert_eq!(
        found(&store, "臺大計中信箱系統維護"),
        1,
        "the whole run matches"
    );
    for part in ["臺大", "計中", "維護", "系統維護"] {
        assert_eq!(
            found(&store, part),
            0,
            "{part} is findable now — good, update F125"
        );
    }
    assert_eq!(found(&store, "本週六凌晨兩點至六點暫停服務"), 1);
    assert_eq!(found(&store, "暫停服務"), 0);
}

#[test]
fn english_is_unaffected() {
    // The tokeniser is right for languages that put spaces between words, which is why it was
    // chosen. Whatever fixes CJK must not cost this.
    let (store, _dir) = with_chinese_mail();
    assert_eq!(found(&store, "ccnoreply"), 1, "the sender is still a word");
}

#[test]
#[ignore = "measurement"]
fn what_can_be_found_in_a_chinese_message() {
    let (store, _dir) = with_chinese_mail();

    // The field clauses are `LIKE '%needle%'` on a column, not FTS — so they are substring
    // matching and should not have the tokeniser's problem at all.
    for needle in ["臺大", "維護", "服務"] {
        let hits = store
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
            .len();
        eprintln!("  subject:{needle:?} -> {hits} hits");
    }

    for needle in [
        "臺大計中信箱系統維護", // the whole subject run
        "臺大",                 // a word inside it
        "計中",
        "維護",
        "本週六凌晨兩點至六點暫停服務",
        "暫停服務",
        "服務",
    ] {
        eprintln!("  search {needle:?} -> {} hits", found(&store, needle));
    }
}
