//! A server folder as a place, in the running window: choosing its row lists what the server
//! holds there and fetches it once, its badge counts it, and an op keeps or lets go of a row by
//! the folders its messages are in.

use super::super::app::App;
use super::super::folder_open::Fetcher;
use super::super::motion::belongs;
use super::folder_tests::{IMAP, folder};
use crate::sync::Ran;
use crate::ui::fixtures::{Seen, click, dispatching, empty, rebuild_into};
use crate::view::{Shell, folder_of, places_with};
use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PROJECTS: &str = "Projects/2026";
const RECEIPTS: &str = "收據";
const OLD: &str = "Old news";

/// A non-Gmail IMAP account with nested folders, one of them unfollowed, and mail in three.
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
    assert_ne!(preset.expected_caps.labels, ServerLabels::Supported);
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
    store
        .put_caps(IMAP, &preset.expected_caps, Utc::now())
        .unwrap();
    let mut old = folder(OLD, Some('/'));
    old.subscription = Subscription::Unsubscribed;
    store
        .put_folders(
            IMAP,
            vec![
                folder("INBOX", Some('/')),
                folder("Projects", Some('/')),
                folder(PROJECTS, Some('/')),
                folder(RECEIPTS, Some('/')),
                old,
            ],
        )
        .unwrap();
    let threads = [
        (PROJECTS, 1, "Q3 plan draft"),
        (PROJECTS, 2, "Budget for the Q3 plan"),
        (RECEIPTS, 3, "Your receipt for September"),
        (OLD, 4, "A newsletter from last year"),
    ]
    .iter()
    .map(|(path, uid, subject)| deliver(&store, IMAP, path, *uid, subject))
    .collect();
    (store, dir, threads)
}

fn remote(path: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: path.to_owned(),
        uidvalidity: 1,
        uid,
    }
}

fn batch(account: AccountId, path: &str) -> Ingest {
    Ingest {
        mailbox: MailboxRef {
            account,
            path: path.to_owned(),
        },
        validity: UidValidity::Same,
        cursor: None,
        messages: vec![],
        flags: vec![],
        labels: vec![],
        label_names: vec![],
        gone: vec![],
    }
}

/// One unread message the server holds in `path`, filed as a header fetch from there files it.
fn deliver(
    store: &SqliteStore,
    account: AccountId,
    path: &str,
    uid: u32,
    subject: &str,
) -> ThreadId {
    let raw = store.blobs().put(&store.connection(), b"x").unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: MessageId::generate(),
        thread,
        account,
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
        mailbox: FolderRoles::default().filed_as(path),
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    let mut ingest = batch(account, path);
    ingest.messages.push(Fetched {
        remote: remote(path, uid),
        key: message.key.clone(),
        raw,
        message,
    });
    store.ingest(account, ingest).unwrap();
    thread
}

/// A fetcher that counts what it is asked for and answers `answer`.
fn counting(answer: Result<&'static str, &'static str>) -> (Fetcher, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let fetcher = Fetcher(Arc::new(move |_store, _account, path: &str, _now| {
        assert!([PROJECTS, RECEIPTS, OLD].contains(&path), "{path}");
        seen.fetch_add(1, Ordering::SeqCst);
        answer
            .map(|text| Ran {
                text: format!("{text} {path}"),
                rejected: false,
                hold: None,
            })
            .map_err(str::to_owned)
    }));
    (fetcher, calls)
}

fn window(store: Arc<SqliteStore>, fetcher: Fetcher) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(App)
        .with_root_context(store)
        .with_root_context(fetcher);
    let seen = rebuild_into(&mut dom);
    (dom, seen)
}

/// Let the fetch and the list query land and redraw, as the window would between frames.
async fn settle(dom: &mut VirtualDom) {
    for _ in 0..12 {
        if tokio::time::timeout(std::time::Duration::from_millis(60), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut NoOpMutations);
    }
}

/// The folder row whose path is `path`: from quire's tree item row that names it to its ⋯.
fn row<'a>(page: &'a str, path: &str, name: &str) -> &'a str {
    let start = page
        .find(&format!("data-place=\"{path}\""))
        .unwrap_or_else(|| panic!("no row for {path}: {page}"));
    let end = page[start..].find(&format!("Actions for {name}")).unwrap() + start;
    &page[start..end]
}

/// The count a row's badge shows: quire's item count, `data-place="item"`.
fn count(row: &str) -> Option<&str> {
    let at = row.find("data-place=\"item\"")?;
    let text = &row[at..];
    let open = text.find('>')? + 1;
    let close = text[open..].find('<')? + open;
    Some(&text[open..close])
}

/// The list pane: from the list bar to the end.
fn list(page: &str) -> &str {
    &page[page.find("class=\"list-bar\"").unwrap()..]
}

#[tokio::test]
async fn choosing_a_folder_lists_what_the_server_holds_there_and_its_badge_counts_it() {
    let (store, _dir, _) = store();
    let (fetcher, _) = counting(Ok("fetched"));
    let (mut dom, seen) = window(store, fetcher);
    let before = dioxus_ssr::render(&dom);
    assert!(
        !list(&before).contains("Q3 plan draft"),
        "listed before it was chosen"
    );
    assert_eq!(
        count(row(&before, PROJECTS, "2026")),
        Some("2"),
        "the badge does not count its two unread threads: {}",
        row(&before, PROJECTS, "2026")
    );
    assert_eq!(count(row(&before, RECEIPTS, RECEIPTS)), Some("1"));
    assert!(
        !before.contains("not here to list"),
        "the old tooltip is still there"
    );

    click(&mut dom, seen.folder(PROJECTS));
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    let listed = list(&page);
    assert!(
        listed.contains("<h2>2026"),
        "the title is not the leaf: {listed}"
    );
    for subject in ["Q3 plan draft", "Budget for the Q3 plan"] {
        assert!(
            listed.contains(subject),
            "{subject} is not listed: {listed}"
        );
    }
    for subject in ["Your receipt for September", "A newsletter from last year"] {
        assert!(!listed.contains(subject), "{subject} is listed: {listed}");
    }
    // The row's own element carries it, beside the place it names.
    let name = page.find("data-place=\"Projects/2026\"").unwrap();
    let current = page[..name].rfind("aria-current=").unwrap();
    assert!(
        page[current..].starts_with("aria-current=\"true\""),
        "the row is not marked as chosen: {}",
        &page[current..name]
    );
    // One account in view: the title names no address.
    assert!(!listed.contains("me@nowhere.example"), "{listed}");
}

#[tokio::test]
async fn opening_fetches_once_per_choice_and_not_again_within_the_minute() {
    let (store, _dir, _) = store();
    let (fetcher, calls) = counting(Ok("fetched"));
    let (mut dom, seen) = window(store, fetcher);
    settle(&mut dom).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "fetched before anything was opened"
    );

    click(&mut dom, seen.folder(PROJECTS));
    settle(&mut dom).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let page = dioxus_ssr::render(&dom);
    assert!(
        list(&page).contains("fetched Projects/2026"),
        "what the fetch said is not in the status line: {}",
        list(&page)
    );

    // Redrawing is not opening.
    dom.mark_dirty(ScopeId::APP);
    dom.render_immediate(&mut NoOpMutations);
    settle(&mut dom).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a re-render fetched");

    click(&mut dom, seen.folder(RECEIPTS));
    settle(&mut dom).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2, "another folder is fetched");

    click(&mut dom, seen.folder(PROJECTS));
    settle(&mut dom).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "fetched again within the minute"
    );

    // An unfollowed folder, drawn by "Show all", opens and fetches too.
    let shown = click(
        &mut dom,
        seen.one("title", "Show the folders you do not follow"),
    );
    click(&mut dom, shown.folder(OLD));
    settle(&mut dom).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let page = dioxus_ssr::render(&dom);
    assert!(
        list(&page).contains("A newsletter from last year"),
        "{}",
        list(&page)
    );
}

#[tokio::test]
async fn a_fetch_that_fails_says_why_in_the_status_line() {
    let (store, _dir, _) = store();
    let (fetcher, calls) = counting(Err("the server has no such folder any more"));
    let (mut dom, seen) = window(store, fetcher);
    click(&mut dom, seen.folder(RECEIPTS));
    settle(&mut dom).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let page = dioxus_ssr::render(&dom);
    let bar = list(&page);
    let status = &bar[bar.find("class=\"status bad\"").expect("no failure line")..];
    assert!(
        status.contains("收據 was not fetched: the server has no such folder any more"),
        "{status}"
    );
    // What was already held is still listed.
    assert!(bar.contains("Your receipt for September"), "{bar}");
}

#[test]
fn an_op_in_a_folder_place_keeps_the_row_while_its_mail_is_still_there() {
    let (store, _dir, threads) = store();
    let places = places_with(&[], &super::folder_places(&store));
    let index = places
        .iter()
        .position(|place| folder_of(place).is_some_and(|m| m.path == PROJECTS))
        .unwrap();
    let mut shell = Shell {
        places,
        ..Shell::default()
    };
    shell.select(index);
    let draft = threads[0];

    // Starring changes nothing about where the server holds it.
    super::super::ops::perform(&store, draft, Op::SetStar(Star::Starred)).unwrap();
    assert!(
        belongs(&store, &shell, draft, Utc::now()),
        "a starred row left"
    );

    // Trashed here, it leaves the folder's list at once (10.11): `InFolder` is held there *and*
    // filed as held, and a trashed message is filed as none of its folders. The server still
    // holds it in the folder until the move is made and seen, and its address stays.
    let trashed = super::super::ops::perform(&store, draft, Op::Trash).unwrap();
    assert!(
        !belongs(&store, &shell, draft, Utc::now()),
        "a trashed row stayed"
    );
    // Undone, as a refused move is, it is back.
    assert!(super::super::ops::take_back(&store, &trashed));
    assert!(
        belongs(&store, &shell, draft, Utc::now()),
        "an undone trash did not come back"
    );

    // Once the server has moved it and a sync has seen it go, it is out of the folder.
    let mut gone = batch(IMAP, PROJECTS);
    gone.gone.push(remote(PROJECTS, 1));
    store.ingest(IMAP, gone).unwrap();
    assert!(
        !belongs(&store, &shell, draft, Utc::now()),
        "a row that left stayed"
    );

    // Another folder's thread was never this place's.
    assert!(!belongs(&store, &shell, threads[2], Utc::now()));
}

/// Writes `target/folder-place.html` and its `-dark` twin: the Work Space with one account's
/// own folders in the sidebar, a folder chosen, and its mail in the list.
#[tokio::test]
#[ignore = "writes target/folder-place*.html for a human or a headless browser to look at"]
async fn render_a_folder_place_to_a_file() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let rows = crate::ui::data::account_rows(&built.store);
    let account = rows.last().unwrap().id;
    let caps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        ..crate::ui::fixtures::gmail_caps()
    };
    built.store.put_caps(account, &caps, Utc::now()).unwrap();
    let listed = |path: &str| Folder {
        account,
        ..folder(path, Some('/'))
    };
    let mut old = listed("Old newsletters");
    old.subscription = Subscription::Unsubscribed;
    let folders = vec![
        listed("INBOX"),
        listed("Projects"),
        listed(PROJECTS),
        listed("Projects/2026/Q3"),
        listed(RECEIPTS),
        listed("旅行/京都"),
        old,
    ];
    built.store.put_folders(account, folders).unwrap();
    for (uid, subject) in [
        (41, "Q3 plan draft, with the open questions"),
        (42, "Budget for the Q3 plan"),
        (43, "Offsite venue: two options"),
    ] {
        deliver(&built.store, account, PROJECTS, uid, subject);
    }
    deliver(
        &built.store,
        account,
        RECEIPTS,
        44,
        "Your receipt for September",
    );
    let space = crate::space::load(&built.dirs.config).current_space();
    let (fetcher, _) = counting(Ok("Up to date:"));
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs)
        .with_root_context(fetcher);
    let seen = rebuild_into(&mut dom);
    click(&mut dom, seen.folder(PROJECTS));
    settle(&mut dom).await;
    let body = dioxus_ssr::render(&dom);
    for (suffix, scheme) in [("", ds::Scheme::Light), ("-dark", ds::Scheme::Dark)] {
        let framed = crate::ui::fixtures::framed(&body, scheme, &space.look);
        crate::ui::fixtures::write_page(
            &format!("folder-place{suffix}"),
            &crate::ui::fixtures::page(&framed, ""),
        );
    }
}
