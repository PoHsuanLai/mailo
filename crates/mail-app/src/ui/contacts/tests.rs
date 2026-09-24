//! The book in the window, against a real store in a temporary directory: the sender card's
//! writes, the sheet's import and export, and how cheap a suggestion is.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::*;
use mail_store::{Origin, SqliteStore, Store};

use super::book;
use super::card::ContactPart;
use crate::ui::fixtures::{ACCOUNT, chord, click, dispatching, rebuild_into, type_into};

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
        account: ACCOUNT,
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
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
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
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?2, '{}', datetime('now'))",
            [ACCOUNT.to_string(), ME.to_owned()],
        )
        .unwrap_or_else(|why| panic!("an account: {why}"));
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Dave', ?3, '\"default\"')",
            [
                IdentityId::generate().to_string(),
                ACCOUNT.to_string(),
                ME.to_owned(),
            ],
        )
        .unwrap_or_else(|why| panic!("an identity: {why}"));
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

#[test]
fn the_fixture_has_each_kind_of_entry_the_tests_rely_on() {
    let (store, _dir) = the_book();
    let every: Vec<String> = store
        .contacts()
        .unwrap_or_default()
        .into_iter()
        .map(|contact| contact.address)
        .collect();
    for address in [ME, ADDED, WRITTEN, HEARD, NO_REPLY] {
        assert!(
            every.iter().any(|one| one == address),
            "{address} not in {every:?}"
        );
    }
    let offered: Vec<String> = book::suggest(store.as_ref(), "da")
        .into_iter()
        .map(|person| person.address)
        .collect();
    assert!(!offered.contains(&ME.to_owned()), "{offered:?}");
    assert!(!offered.contains(&NO_REPLY.to_owned()), "{offered:?}");
}

/// The sender card, alone, on `email`.
#[component]
fn Card(email: String, name: String) -> Element {
    rsx! { ContactPart { email, name } }
}

pub(super) fn card(store: &Arc<SqliteStore>, email: &str, name: &str) -> VirtualDom {
    VirtualDom::new_with_props(
        Card,
        CardProps {
            email: email.to_owned(),
            name: name.to_owned(),
        },
    )
    .with_root_context(store.clone())
}

fn named(store: &SqliteStore, address: &str) -> Option<(Option<String>, Origin)> {
    store
        .contact(address)
        .unwrap_or_else(|why| panic!("read {address}: {why}"))
        .map(|contact| (contact.name, contact.origin))
}

#[tokio::test]
async fn the_sender_card_adds_renames_and_forgets() {
    dispatching();
    let (store, _dir) = the_book();
    assert_eq!(
        named(&store, HEARD),
        Some((Some("Dana Okafor".to_owned()), Origin::History))
    );
    let mut dom = card(&store, HEARD, "Dana Okafor");
    let seen = rebuild_into(&mut dom);

    // Add: the field opens on the name the mail gave, and Enter keeps what was typed.
    let seen = click(
        &mut dom,
        seen.one("aria-label", &format!("Add to contacts: {HEARD}")),
    );
    let field = seen.one("value", "Dana Okafor");
    type_into(&mut dom, field, "Dana O.");
    let seen = chord(&mut dom, "Enter", Modifiers::empty(), field);
    assert_eq!(
        named(&store, HEARD),
        Some((Some("Dana O.".to_owned()), Origin::Manual))
    );

    // Edit: now the user's own entry, so the action says so, and Save keeps the new name.
    let seen = click(
        &mut dom,
        seen.one("aria-label", &format!("Edit name: {HEARD}")),
    );
    let field = seen.one("value", "Dana O.");
    type_into(&mut dom, field, "Dana Okafor-Reyes");
    let seen = click(
        &mut dom,
        seen.one("aria-label", &format!("Save the name for {HEARD}")),
    );
    assert_eq!(
        named(&store, HEARD),
        Some((Some("Dana Okafor-Reyes".to_owned()), Origin::Manual))
    );

    // Forget: gone from the book, and the card offers to add it again.
    click(&mut dom, seen.one("aria-label", &format!("Forget {HEARD}")));
    assert_eq!(named(&store, HEARD), None);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Add to contacts"), "{page}");
    assert!(!page.contains(">Forget<"), "{page}");
}

#[tokio::test]
async fn the_sender_card_adds_an_address_the_book_has_never_seen() {
    dispatching();
    let (store, _dir) = the_book();
    let stranger = "someone.new@example.test";
    assert_eq!(named(&store, stranger), None);
    let mut dom = card(&store, stranger, "");
    let seen = rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(!page.contains(">Forget<"), "nothing to forget yet: {page}");
    let seen = click(
        &mut dom,
        seen.one("aria-label", &format!("Add to contacts: {stranger}")),
    );
    let field = seen.one("value", "");
    type_into(&mut dom, field, "Sam Novak");
    chord(&mut dom, "Enter", Modifiers::empty(), field);
    assert_eq!(
        named(&store, stranger),
        Some((Some("Sam Novak".to_owned()), Origin::Manual))
    );
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
