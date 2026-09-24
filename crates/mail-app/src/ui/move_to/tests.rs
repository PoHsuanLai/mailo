//! "Move to…" in the running window: the menu offers the account's own folders and nothing
//! else, a pick files the conversation through `Op::File` with the undo every op has, and a row
//! dropped on a folder in the sidebar is filed the same way.

use std::sync::Arc;

use chrono::Utc;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

use super::{destinations, folder_label, items};
use crate::ui::fixtures::{
    FakePointer, INSIDE_THE_SHELL, Seen, chord, click, dispatching, empty, pointer, rebuild_into,
};
use crate::ui::folder_open::Fetcher;
use crate::view::folder_filter;

const IMAP: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"));
const FROM: &str = "Projects/2026";
const TO: &str = "Projects";

fn folder(path: &str, special: Option<SpecialUse>, holds: Holds) -> Folder {
    Folder {
        account: IMAP,
        path: path.to_owned(),
        delimiter: Some('/'),
        special,
        subscription: Subscription::Subscribed,
        holds,
    }
}

/// A non-Gmail IMAP account with folders of its own, special-use ones, a level that holds only
/// folders, a label of the user's, and two conversations in `FROM`.
fn store() -> (Arc<SqliteStore>, tempfile::TempDir, Vec<ThreadId>) {
    let (store, dir) = empty();
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual("me@nowhere.example", &manual, Utc::now());
    // As a first sync finds a server that files: its Archive folder is named, so a move to a
    // folder is something the server is told.
    let caps = AccountCaps {
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        ..preset.expected_caps.clone()
    };
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                IMAP.to_string(),
                preset.plan.address.clone(),
                serde_json::to_string(&preset.plan).unwrap(),
                Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();
    store.put_caps(IMAP, &caps, Utc::now()).unwrap();
    store
        .put_folders(
            IMAP,
            vec![
                folder("INBOX", None, Holds::Mail),
                folder("Sent", Some(SpecialUse::Sent), Holds::Mail),
                folder("Trash", Some(SpecialUse::Trash), Holds::Mail),
                folder("Lists", None, Holds::FoldersOnly),
                folder(TO, None, Holds::Mail),
                folder(FROM, None, Holds::Mail),
                folder("收據", None, Holds::Mail),
            ],
        )
        .unwrap();
    store
        .apply(
            IMAP,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(Label {
                    id: LabelId::generate(),
                    account: IMAP,
                    name: "travel".to_owned(),
                    color: None,
                    origin: LabelOrigin::User,
                })],
            },
        )
        .unwrap();
    let threads = [(1, "Q3 plan draft"), (2, "Budget for the Q3 plan")]
        .iter()
        .map(|(uid, subject)| deliver(&store, *uid, subject))
        .collect();
    (store, dir, threads)
}

/// One unread message the server holds in `FROM`.
fn deliver(store: &SqliteStore, uid: u32, subject: &str) -> ThreadId {
    let raw = store.blobs().put(&store.connection(), b"x").unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: MessageId::generate(),
        thread,
        account: IMAP,
        key: MessageKey::Rfc(format!("m{uid}@example.test")),
        date: Utc::now() - chrono::TimeDelta::hours(i64::from(uid)),
        from: Address {
            name: None,
            email: "someone@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: FolderRoles::default().filed_as(FROM),
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    let ingest = Ingest {
        mailbox: MailboxRef {
            account: IMAP,
            path: FROM.to_owned(),
        },
        validity: UidValidity::Same,
        cursor: None,
        messages: vec![Fetched {
            remote: RemoteRef::Imap {
                mailbox: FROM.to_owned(),
                uidvalidity: 1,
                uid,
            },
            key: message.key.clone(),
            raw,
            message,
        }],
        flags: vec![],
        labels: vec![],
        label_names: vec![],
        gone: vec![],
    };
    store.ingest(IMAP, ingest).unwrap();
    thread
}

/// How many conversations the folder at `path` lists: its own place's filter, asked of the store.
fn in_folder(store: &SqliteStore, path: &str) -> u64 {
    let place = folder_filter(&MailboxRef {
        account: IMAP,
        path: path.to_owned(),
    });
    store.count(&place, Utc::now()).unwrap()
}

/// How many conversations carry the label a move into `path` files under.
fn filed_in(store: &SqliteStore, path: &str) -> u64 {
    let label = folder_label(store, IMAP, path).unwrap();
    store.count(&Filter::HasLabel(label), Utc::now()).unwrap()
}

/// Filing moves still waiting for the server.
fn queued_moves(store: &SqliteStore) -> usize {
    let later = Utc::now() + chrono::TimeDelta::days(1);
    store
        .outbox_due(IMAP, later)
        .unwrap()
        .iter()
        .filter(|entry| matches!(entry.op, ProtoOp::File { .. }))
        .count()
}

#[test]
fn the_menu_offers_the_accounts_own_folders_and_nothing_else() {
    let (store, _dir, _) = store();
    let offered: Vec<String> = destinations(&store, IMAP)
        .into_iter()
        .map(|d| d.path)
        .collect();
    assert_eq!(offered, [TO, FROM, "收據"]);
    let keys: Vec<String> = items(&destinations(&store, IMAP))
        .into_iter()
        .map(|item| item.key)
        .collect();
    assert_eq!(keys, offered);
    // A POP3-like account, with no listing at all, has nothing to offer.
    let (bare, _dir) = crate::ui::fixtures::seeded();
    assert!(destinations(&bare, crate::ui::fixtures::ACCOUNT).is_empty());
}

fn window(store: Arc<SqliteStore>) -> (VirtualDom, Seen) {
    dispatching();
    let fetcher = Fetcher(Arc::new(|_, _, path: &str, _| {
        Ok(crate::sync::Ran {
            text: format!("fetched {path}"),
            rejected: false,
            hold: None,
        })
    }));
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store)
        .with_root_context(fetcher);
    let seen = rebuild_into(&mut dom);
    (dom, seen)
}

async fn settle(dom: &mut VirtualDom) {
    for _ in 0..12 {
        let quiet = std::time::Duration::from_millis(60);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut NoOpMutations);
    }
}

/// Open `FROM`'s place, and return what the list drew.
async fn open_the_folder(dom: &mut VirtualDom, seen: &Seen) -> Seen {
    let mut drawn = click(dom, seen.one("data-folder", FROM));
    for _ in 0..12 {
        let quiet = std::time::Duration::from_millis(60);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        drawn = drawn.merge(more);
    }
    drawn
}

#[tokio::test]
async fn moving_to_a_folder_files_it_there_and_undo_brings_it_back() {
    let (store, _dir, _) = store();
    let (mut dom, seen) = window(store.clone());
    let listed = open_the_folder(&mut dom, &seen).await;
    let (from_before, to_before) = (in_folder(&store, FROM), filed_in(&store, TO));
    assert_eq!((from_before, to_before), (2, 0));

    let rows = listed.all("aria-label", "Move to…");
    assert_eq!(rows.len(), 2, "each row offers Move to…");
    let menu = click(&mut dom, rows[0]);
    let page = dioxus_ssr::render(&dom);
    let drawn = &page[page
        .find("class=\"row-menu move-menu")
        .expect("the menu did not open")..];
    // The menu is the last thing in its row.
    let drawn = &drawn[..drawn.find("</li>").unwrap_or(drawn.len())];
    for path in [TO, FROM, "收據"] {
        assert!(
            drawn.contains(&format!("<span>{path}</span>")),
            "{path} is not offered"
        );
    }
    for not in ["INBOX", "Sent", "Trash", "Lists", "travel"] {
        assert!(
            !drawn.contains(&format!("<span>{not}</span>")),
            "{not} is offered: {drawn}"
        );
    }

    // The first folder is the cursor's; Enter takes it.
    chord(
        &mut dom,
        "Enter",
        Modifiers::empty(),
        menu.one("class", "fmenu"),
    );
    settle(&mut dom).await;
    assert_eq!(
        in_folder(&store, FROM),
        from_before - 1,
        "it is still listed in {FROM}"
    );
    assert_eq!(
        filed_in(&store, TO),
        to_before + 1,
        "it was not filed in {TO}"
    );
    assert_eq!(
        queued_moves(&store),
        1,
        "the server was not asked to move it"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Moved to folder"), "no toast: {page}");

    // Ctrl Z: back where it was, and the server never hears of it.
    chord(
        &mut dom,
        "z",
        Modifiers::CONTROL,
        dioxus_core::ElementId(INSIDE_THE_SHELL as usize),
    );
    settle(&mut dom).await;
    assert_eq!(
        in_folder(&store, FROM),
        from_before,
        "undo did not bring it back"
    );
    assert_eq!(filed_in(&store, TO), to_before);
    assert_eq!(
        queued_moves(&store),
        0,
        "the move is still queued after its undo"
    );
}

#[tokio::test]
async fn a_row_dropped_on_a_folder_is_filed_there() {
    let (store, _dir, _) = store();
    let (mut dom, seen) = window(store.clone());
    let listed = open_the_folder(&mut dom, &seen).await;
    let (from_before, to_before) = (in_folder(&store, "收據"), filed_in(&store, "收據"));
    let moving_before = in_folder(&store, FROM);
    let at = |x: f64, y: f64, held: bool| FakePointer {
        client: (x, y),
        offset: (4.0, 4.0),
        held,
    };
    let row = listed.all("aria-label", "Move to…")[0];
    let target = seen.one("data-folder", "收據");
    pointer(&mut dom, "pointerdown", row, at(420.0, 120.0, true));
    pointer(&mut dom, "pointermove", row, at(300.0, 160.0, true));
    pointer(&mut dom, "pointermove", row, at(120.0, 300.0, true));
    let lit = dioxus_ssr::render(&dom);
    assert!(
        lit.contains("fold-row can-drop"),
        "no folder offers to take it: {lit}"
    );
    pointer(&mut dom, "pointerenter", target, at(120.0, 300.0, true));
    pointer(&mut dom, "pointerup", target, at(120.0, 300.0, false));
    settle(&mut dom).await;

    assert_eq!(
        in_folder(&store, FROM),
        moving_before - 1,
        "it is still in {FROM}"
    );
    assert_eq!(filed_in(&store, "收據"), to_before + 1, "it was not filed");
    // The target lists it once the server has moved it; nothing claims that before.
    assert_eq!(in_folder(&store, "收據"), from_before);
    let page = dioxus_ssr::render(&dom);
    let left = page
        .find("fold-row can-drop")
        .map(|at| &page[at.saturating_sub(300)..at + 50]);
    assert!(left.is_none(), "the drag outlived the drop: {left:?}");
}

#[tokio::test]
#[ignore = "writes target/move-to.html and its -dark twin to look at"]
async fn render_the_move_to_menu_to_a_file() {
    let (store, _dir, _) = store();
    let (mut dom, seen) = window(store);
    let listed = open_the_folder(&mut dom, &seen).await;
    click(&mut dom, listed.all("aria-label", "Move to…")[0]);
    crate::ui::fixtures::dump("move-to", &dioxus_ssr::render(&dom));
}
