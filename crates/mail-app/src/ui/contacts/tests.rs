//! The book in the window, against a real store in a temporary directory: the sender card's
//! writes, the sheet's import and export, and how cheap a suggestion is.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{Origin, SqliteStore, Store};

use super::book;
use crate::ui::fixtures::acct_account;

/// The user's own address. It is on every message they sent, so the book learns it as theirs.
pub(in crate::ui) const ME: &str = "dave.me@example.test";
/// Added by hand and never written to.
pub(in crate::ui) const ADDED: &str = "dara.quinn@example.test";
/// Written to twice, so ranked above anyone only heard from.
pub(in crate::ui) const WRITTEN: &str = "daniel@example.test";
/// Heard from once.
pub(in crate::ui) const HEARD: &str = "dana@example.test";
/// A no-reply sender, never written to: never suggested.
pub(in crate::ui) const NO_REPLY: &str = "no-reply@daily.example.test";

fn at(day: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + day * 86_400, 0)
        .single()
        .unwrap_or_else(|| panic!("a real instant"))
}

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

/// Message `n`, in `mailbox`, from `from` to `to`, `day` days in.
fn message(n: u128, day: i64, mailbox: MailboxRole, from: Address, to: Vec<Address>) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1_000_000)),
        account: acct_account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(day),
        from,
        reply_to: vec![],
        to,
        cc: vec![],
        bcc: vec![],
        subject: format!("message {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn deliver(store: &SqliteStore, m: &Message) {
    let path = if m.mailbox == MailboxRole::Sent {
        "Sent"
    } else {
        "INBOX"
    };
    let uid = (m.id.as_uuid().as_u128() % 1_000_000) as u32;
    store
        .ingest(
            acct_account(),
            Ingest {
                mailbox: MailboxRef {
                    account: acct_account(),
                    path: path.to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: path.to_owned(),
                        uidvalidity: 7,
                        uid,
                    },
                    key: m.key.clone(),
                    raw: BlobId::generate(),
                    message: m.clone(),
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap_or_else(|why| panic!("ingest {}: {why}", m.subject));
}

/// A store whose book holds one of each: an entry added by hand, one written to, one heard
/// from, a no-reply sender and the user's own address — every one of them matching "da".
pub(in crate::ui) fn the_book() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap_or_else(|why| panic!("a temp dir: {why}"));
    let store = SqliteStore::in_memory(dir.path()).unwrap_or_else(|why| panic!("a store: {why}"));
    {
        mail_store::testing::seed_account(&store, acct_account(), ME);
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            ME,
            Some("Dave"),
        );
    }
    let me = || addr(Some("Dave"), ME);
    let sent = |n, day, to: &str, name| {
        message(n, day, MailboxRole::Sent, me(), vec![addr(Some(name), to)])
    };
    let got = |n, day, from: Address| message(n, day, MailboxRole::Inbox, from, vec![me()]);
    deliver(&store, &sent(1, 1, WRITTEN, "Daniel Brook"));
    deliver(&store, &sent(2, 2, WRITTEN, "Daniel Brook"));
    deliver(&store, &got(3, 3, addr(Some("Dana Okafor"), HEARD)));
    deliver(&store, &got(4, 4, addr(Some("Daily Digest"), NO_REPLY)));
    // A copy the user sent themselves.
    deliver(&store, &got(5, 5, me()));
    store
        .put_contact(ADDED, Some("Dara Quinn"), &Origin::Manual)
        .unwrap_or_else(|why| panic!("a contact by hand: {why}"));
    (Arc::new(store), dir)
}

fn named(store: &SqliteStore, address: &str) -> Option<(Option<String>, Origin)> {
    store
        .contact(address)
        .unwrap_or_else(|why| panic!("read {address}: {why}"))
        .map(|contact| (contact.name, contact.origin))
}

/// A card as another client exports it: vCard 3.0, folded, with a name in Chinese.
const CARD: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:王小明\r\nN:王;小明;;;\r\n\
                    EMAIL;TYPE=INTERNET:xiaoming.wang@example.test\r\nEND:VCARD\r\n";

#[test]
fn a_name_in_chinese_survives_import_then_export() {
    let (store, _dir) = the_book();
    let out = tempfile::tempdir().unwrap_or_else(|why| panic!("a temp dir: {why}"));
    let before = store.contacts().unwrap_or_default().len();

    let said =
        book::import(store.as_ref(), CARD.as_bytes()).unwrap_or_else(|why| panic!("import: {why}"));
    assert!(said.contains("imported 1 addresses from 1 cards"), "{said}");
    assert_eq!(store.contacts().unwrap_or_default().len(), before + 1);
    assert_eq!(
        named(&store, "xiaoming.wang@example.test"),
        Some((Some("王小明".to_owned()), Origin::Manual))
    );

    let first =
        book::export(store.as_ref(), out.path()).unwrap_or_else(|why| panic!("export: {why}"));
    assert_eq!(first, out.path().join(book::EXPORT_NAME));
    let text = std::fs::read_to_string(&first).unwrap_or_else(|why| panic!("read back: {why}"));
    assert!(text.contains("FN:王小明"), "{text}");
    assert!(text.contains("xiaoming.wang@example.test"), "{text}");

    // A second export never writes over the first.
    let second = book::export(store.as_ref(), out.path())
        .unwrap_or_else(|why| panic!("export again: {why}"));
    assert_ne!(second, first);
    assert_eq!(
        std::fs::read_to_string(&first).unwrap_or_default(),
        text,
        "the first file changed"
    );

    // And the file read back into an empty book gives the same name.
    let (fresh, _fresh_dir) = the_book();
    book::import(fresh.as_ref(), text.as_bytes()).unwrap_or_else(|why| panic!("re-import: {why}"));
    assert_eq!(
        named(&fresh, "xiaoming.wang@example.test"),
        Some((Some("王小明".to_owned()), Origin::Manual))
    );
}

#[test]
fn the_sheet_filters_by_the_start_of_any_word_and_lists_everyone() {
    let (store, _dir) = the_book();
    let addresses = |filter: &str| -> Vec<String> {
        book::rows(store.as_ref(), filter)
            .unwrap_or_else(|why| panic!("rows: {why}"))
            .into_iter()
            .map(|row| row.address)
            .collect()
    };
    // The sheet is the whole book: the no-reply sender and the user's own address are listed,
    // and say why they are not suggested.
    let all = addresses("");
    assert_eq!(all.len(), store.contacts().unwrap_or_default().len());
    let rows = book::rows(store.as_ref(), "").unwrap_or_default();
    let quiet = |address: &str| {
        rows.iter()
            .find(|row| row.address == address)
            .and_then(|row| row.quiet)
    };
    assert_eq!(quiet(ME), Some("your address"));
    assert_eq!(quiet(NO_REPLY), Some("not suggested until you write"));
    assert_eq!(quiet(ADDED), None);

    const CASES: &[(&str, &[&str])] = &[
        ("quinn", &[ADDED]),
        ("QUI", &[ADDED]),
        ("dara q", &[ADDED]),
        ("okafor", &[HEARD]),
        ("uinn", &[]),
        ("daily.example", &[NO_REPLY]),
    ];
    for (filter, want) in CASES {
        let want: Vec<String> = want.iter().map(|one| (*one).to_owned()).collect();
        assert_eq!(addresses(filter), want, "filter {filter:?}");
    }
}

/// Timing, not correctness, so not run by default: `cargo test -p mail-app suggestion_is_cheap
/// -- --ignored --nocapture`. What the composer's decision to ask on every keystroke rests on.
#[test]
#[ignore = "prints how long a suggestion takes at ten thousand contacts"]
fn a_suggestion_is_cheap_enough_for_every_keystroke() {
    let (store, _dir) = the_book();
    let mut held = 0;
    for size in [1_000, 10_000] {
        for n in held..size {
            store
                .put_contact(
                    &format!("person{n}@host{}.example.test", n % 50),
                    Some(&format!("Person {n} Surname{}", n % 300)),
                    &Origin::Manual,
                )
                .unwrap_or_else(|why| panic!("contact {n}: {why}"));
        }
        held = size;
        for typed in ["d", "pe", "person 12", "surname2", "host7.example", "zz"] {
            let started = std::time::Instant::now();
            let rounds = 50u32;
            for _ in 0..rounds {
                let _ = book::suggest(store.as_ref(), typed);
            }
            println!(
                "{size:>6} {typed:>14}: {:?} per suggestion",
                started.elapsed() / rounds
            );
        }
    }
}
