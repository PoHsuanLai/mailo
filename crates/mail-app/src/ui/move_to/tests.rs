//! "Move to…" in the running window: the menu offers the account's own folders and nothing
//! else, a pick files the conversation through `Op::File` with the undo every op has, and a row
//! dropped on a folder in the sidebar is filed the same way.

use std::sync::Arc;

use chrono::Utc;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_core::{SqliteStore, Store};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;

use super::{destinations, folder_label, items};
use crate::ui::fixtures::{
    FakePointer, INSIDE_THE_SHELL, Seen, chord, click, dispatching, empty, listed_subjects,
    menu_names, open_row_menu, pick_named, pointer, rebuild_into, row_named,
};
use crate::ui::folder_open::Fetcher;
use crate::ui::view::folder_filter;

fn acct_imap() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"))
}
const FROM: &str = "Projects/2026";
const TO: &str = "Projects";

fn folder(path: &str, special: Option<SpecialUse>, holds: Holds) -> Folder {
    Folder {
        account: acct_imap(),
        path: path.to_owned(),
        delimiter: Some('/'),
        special,
        subscription: Subscription::Subscribed,
        holds,
    }
}

/// A non-Gmail acct_imap() account with folders of its own, special-use ones, a level that holds only
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
    mail_store::testing::seed_account_plan(
        &store,
        acct_imap(),
        &preset.plan.address,
        &preset.plan,
        Some(Utc::now()),
    );
    store.put_caps(acct_imap(), &caps, Utc::now()).unwrap();
    store
        .put_folders(
            acct_imap(),
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
            acct_imap(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(Label {
                    id: LabelId::generate(),
                    account: acct_imap(),
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
    let raw = store.blobs().put(b"x").unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: MessageId::generate(),
        thread,
        account: acct_imap(),
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
            account: acct_imap(),
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
    store.ingest(acct_imap(), ingest).unwrap();
    thread
}

/// How many conversations the folder at `path` lists: its own place's filter, asked of the store.
fn in_folder(store: &SqliteStore, path: &str) -> u64 {
    let place = folder_filter(&MailboxRef {
        account: acct_imap(),
        path: path.to_owned(),
    });
    store.count(&place, Utc::now()).unwrap()
}

/// How many conversations carry the label a move into `path` files under.
fn filed_in(store: &SqliteStore, path: &str) -> u64 {
    let label = folder_label(store, acct_imap(), path).unwrap();
    store.count(&Filter::HasLabel(label), Utc::now()).unwrap()
}

/// Filing moves still waiting for the server.
fn queued_moves(store: &SqliteStore) -> usize {
    let later = Utc::now() + chrono::TimeDelta::days(1);
    store
        .outbox_due(acct_imap(), later)
        .unwrap()
        .iter()
        .filter(|entry| matches!(entry.op, ProtoOp::File { .. }))
        .count()
}

#[test]
fn the_menu_offers_the_accounts_own_folders_and_nothing_else() {
    let (store, _dir, _) = store();
    let offered: Vec<String> = destinations(&store, acct_imap())
        .into_iter()
        .map(|d| d.path)
        .collect();
    assert_eq!(offered, [TO, FROM, "收據"]);
    let keys: Vec<String> = items(&destinations(&store, acct_imap()))
        .into_iter()
        .map(|item| item.key)
        .collect();
    assert_eq!(keys, offered);
    // A POP3-like account, with no listing at all, has nothing to offer.
    let (bare, _dir) = crate::ui::fixtures::seeded();
    assert!(destinations(&bare, crate::ui::fixtures::acct_account()).is_empty());
}

fn window(store: Arc<SqliteStore>) -> (VirtualDom, Seen) {
    dispatching();
    let fetcher = Fetcher(Arc::new(|_, account, _, _| {
        Ok(mail_core::sync::report::PassEnd::Finished(
            mail_core::sync::report::AccountReport {
                account,
                address: "me@nowhere.example".to_owned(),
                counts: mail_core::sync::report::Counts::default(),
                trouble: Vec::new(),
            },
        ))
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
    let mut drawn = click(dom, seen.folder(FROM));
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

    let subjects = listed_subjects(&dioxus_ssr::render(&dom));
    assert_eq!(subjects.len(), 2, "the folder lists both conversations");
    let opened = open_row_menu(&mut dom, &listed, &subjects[0]);
    assert!(
        menu_names(&dioxus_ssr::render(&dom)).contains(&"Move to…".to_owned()),
        "the row's menu offers no Move to…"
    );
    let picked = pick_named(&mut dom, &opened, "Move to…").await;
    let menu = floated(&mut dom, picked).await;
    let page = dioxus_ssr::render(&dom);
    // quire's pick list floats where the row's menu stood, in the root's overlay, not in the row.
    let drawn = &page[page
        .find("class=\"ds-pick-list")
        .expect("the picker did not open")..];
    for path in [TO, FROM, "收據"] {
        assert!(
            drawn.contains(&format!(">{path}</b>")),
            "{path} is not offered"
        );
    }
    for not in ["INBOX", "Sent", "Trash", "Lists", "travel"] {
        assert!(
            !drawn.contains(&format!(">{not}</b>")),
            "{not} is offered: {drawn}"
        );
    }

    // The first folder is the list's selection; Enter takes it.
    chord(
        &mut dom,
        "Enter",
        Modifiers::empty(),
        // The list and its field are both named "Move to"; the keys go to the field.
        *menu.all("aria-label", "Move to").last().expect("the field"),
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

    // ⌘Z: back where it was, and the server never hears of it.
    chord(
        &mut dom,
        "z",
        crate::ui::fixtures::PRIMARY,
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

/// Draw until the menu a click opened has floated into the root's overlay: quire's menu is
/// placed a frame after it is asked for.
async fn floated(dom: &mut VirtualDom, mut drawn: Seen) -> Seen {
    for _ in 0..8 {
        if drawn.get("aria-label", "Move to").is_some()
            || tokio::time::timeout(std::time::Duration::from_millis(100), dom.wait_for_work())
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

/// The folders of the moves still waiting for the server, oldest first.
fn queued_folders(store: &SqliteStore) -> Vec<String> {
    let later = Utc::now() + chrono::TimeDelta::days(1);
    store
        .outbox_due(acct_imap(), later)
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry.op {
            ProtoOp::File { folder, .. } => Some(folder),
            _ => None,
        })
        .collect()
}

/// Filed into `path`, as a pick in the menu does it, with the undo it hands the toast.
fn file(store: &SqliteStore, thread: ThreadId, path: &str) -> mail_core::undo::Undo {
    let label = folder_label(store, acct_imap(), path).unwrap();
    crate::ui::ops::perform(store, thread, Op::File(label)).expect("the move was not made")
}

#[test]
fn a_move_taken_back_with_nothing_after_it_never_reaches_the_server() {
    let (store, _dir, threads) = store();
    let moved = file(&store, threads[0], TO);
    // Another conversation's move, queued later, is not this one's business.
    file(&store, threads[1], "收據");
    assert_eq!(queued_folders(&store), [TO, "收據"]);

    assert!(crate::ui::ops::take_back(&store, &moved));
    assert_eq!(queued_folders(&store), ["收據"], "the move is still queued");
    assert_eq!(filed_in(&store, TO), 0);
}

#[test]
fn a_move_with_a_later_one_on_the_same_conversation_stays_queued_when_taken_back() {
    let (store, _dir, threads) = store();
    let first = file(&store, threads[0], TO);
    file(&store, threads[0], "收據");
    assert_eq!(queued_folders(&store), [TO, "收據"]);

    assert!(crate::ui::ops::take_back(&store, &first));
    // Withdrawing the first would write the state from before it under the second, which was
    // made on top of it. Both go, in the order they were made, and the second is where it ends.
    assert_eq!(
        queued_folders(&store),
        [TO, "收據"],
        "a move with a later one on the same messages was withdrawn"
    );
    assert_eq!(filed_in(&store, "收據"), 1, "the later move was undone");
    assert_eq!(
        filed_in(&store, TO),
        0,
        "the undo did not take the first back"
    );
    assert_eq!(
        in_folder(&store, FROM),
        1,
        "the later move left it in {FROM}"
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
    let row = row_named(&listed, &listed_subjects(&dioxus_ssr::render(&dom))[0]);
    let target = seen.folder("收據");
    pointer(&mut dom, "pointerdown", row, at(420.0, 120.0, true));
    pointer(&mut dom, "pointermove", row, at(300.0, 160.0, true));
    pointer(&mut dom, "pointermove", row, at(120.0, 300.0, true));
    let lit = dioxus_ssr::render(&dom);
    assert!(folder_accepts(&lit), "no folder offers to take it: {lit}");
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
    assert!(!folder_accepts(&page), "the drag outlived the drop: {page}");
}

/// Whether a folder's row (quire's `Row`, carrying `data-place`) is drawn as taking the row
/// being dragged.
fn folder_accepts(page: &str) -> bool {
    page.match_indices("data-drop=\"accepts\"").any(|(at, _)| {
        let tag_end = page[at..].find('>').map_or(page.len(), |end| at + end);
        page[at..tag_end].contains("data-place=\"")
    })
}

#[tokio::test]
#[ignore = "writes target/move-to.html and its -dark twin to look at"]
async fn render_the_move_to_menu_to_a_file() {
    let (store, _dir, _) = store();
    let (mut dom, seen) = window(store);
    let listed = open_the_folder(&mut dom, &seen).await;
    let subjects = listed_subjects(&dioxus_ssr::render(&dom));
    let opened = open_row_menu(&mut dom, &listed, &subjects[0]);
    pick_named(&mut dom, &opened, "Move to…").await;
    crate::ui::fixtures::dump("move-to", &dioxus_ssr::render(&dom));
}
