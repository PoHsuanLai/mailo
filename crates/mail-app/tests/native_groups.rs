//! Contact groups on the real window (Blitz, through `ds_native::Harness`): typing a group's name
//! in To offers the group, and choosing it puts its members on the message.
//!
//! The window is opened over a store seeded in a `TempDir` and handed no directories, so it
//! writes no file anywhere; nothing here reads or touches the real mail store or config. The
//! account's plan is empty, so the window's poll reaches no server.

use ds::{Key, Point};
use ds_native::{FocusFallback, Harness, HarnessConfig, NetPolicy, PrintOutcome, Viewport};
use mail_domain::*;
use mail_store::{Group, GroupHome, GroupId, Origin, SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The group's members, as the book names them.
const MEMBERS: [(&str, &str); 2] = [
    ("Rosalind Franklin", "rosalind@example.test"),
    ("Barbara Liskov", "barbara@example.test"),
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// One account, the two members in the book, and "Reading circle" holding them and a member
/// that names a card nobody here has.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    for (name, address) in MEMBERS {
        store
            .put_contact(address, Some(name), &Origin::Manual)
            .unwrap();
    }
    let uid = "urn:uuid:7a1d0c3e-0000-4000-8000-0000000000c1";
    store
        .put_group(&Group {
            id: GroupId::local(uid),
            uid: Some(uid.to_owned()),
            name: "Reading circle".to_owned(),
            members: vec![
                format!("mailto:{}", MEMBERS[0].1),
                format!("mailto:{}", MEMBERS[1].1),
                "urn:uuid:ffffffff-0000-4000-8000-000000000000".to_owned(),
            ],
            home: GroupHome::Local,
        })
        .unwrap();
    Arc::new(store)
}

fn open() -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // Every window here has a print dialog of its own: none may open the system's.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_contexts(contexts);
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    (harness, dir)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// The rows above the body, where the chips are.
const TO: &str = ".c-props";

#[test]
fn a_group_typed_in_to_is_offered_and_choosing_it_adds_its_members() {
    let (mut harness, _dir) = open();
    harness.key(Key::Char('c'));
    harness.advance(ms(500));
    harness.click(centre(&harness, ".c-pin input"));
    harness.advance(ms(200));
    let before = harness.text_of(TO).unwrap_or_default();
    for (name, _) in MEMBERS {
        assert!(!before.contains(name), "{name} is on the message already");
    }

    for c in "reading".chars() {
        harness.key(Key::Char(c));
    }
    harness.advance(ms(300));
    let html = harness.html();
    assert!(html.contains("Reading circle"), "no group offered:\n{html}");
    assert!(
        html.contains("Group · 2 people · 1 not found"),
        "the offer says who it adds:\n{html}"
    );

    // The group is the first row, and the menu's cursor starts on it.
    harness.key(Key::Enter);
    harness.advance(ms(300));
    let after = harness.text_of(TO).unwrap_or_default();
    for (name, _) in MEMBERS {
        assert!(
            after.contains(name),
            "{name} is not on the message: {after:?}"
        );
    }
    assert!(
        !harness.html().contains("Group · 2 people"),
        "the menu stayed open"
    );
}
