//! A server folder as a place, in the running window: choosing its row lists what the server
//! holds there and fetches it once, its badge counts it, and an op keeps or lets go of a row by
//! the folders its messages are in.

use super::super::app::App;
use super::super::folder_open::Fetcher;
use super::super::motion::belongs;
use super::folder_tests::{acct_imap, folder};
use crate::ui::fixtures::{Seen, click, dispatching, empty, rebuild_into};
use crate::ui::view::{Shell, folder_of, places_with};
use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use ds::prelude::*;
use mail_core::fetch::FolderFetch;
use mail_core::sync::report::{AccountReport, Counts, PassEnd};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PROJECTS: &str = "Projects/2026";
const RECEIPTS: &str = "收據";
const OLD: &str = "Old news";

/// A non-Gmail acct_imap() account with nested folders, one of them unfollowed, and mail in three.
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
                acct_imap().to_string(),
                preset.plan.address.clone(),
                serde_json::to_string(&preset.plan).unwrap(),
                Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();
    store
        .put_caps(acct_imap(), &preset.expected_caps, Utc::now())
        .unwrap();
    let mut old = folder(OLD, Some('/'));
    old.subscription = Subscription::Unsubscribed;
    store
        .put_folders(
            acct_imap(),
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
    .map(|(path, uid, subject)| deliver(&store, acct_imap(), path, *uid, subject))
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
        account: account.clone(),
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
    let mut ingest = batch(account.clone(), path);
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
fn counting(answer: Result<(), &'static str>) -> (Fetcher, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let fetcher = Fetcher(Arc::new(move |_store, account, path: &str, _now| {
        assert!([PROJECTS, RECEIPTS, OLD].contains(&path), "{path}");
        seen.fetch_add(1, Ordering::SeqCst);
        answer
            .map(|()| {
                PassEnd::Finished(AccountReport {
                    account,
                    address: "me@nowhere.example".to_owned(),
                    counts: Counts::default(),
                    trouble: Vec::new(),
                })
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

/// Where the on-demand fetch of `path` on the account stands, as the window's fetching has it.
fn folder_state(dom: &mut VirtualDom, path: &str) -> FolderFetch {
    dom.in_scope(ScopeId::APP, || {
        consume_context::<super::super::fetching::Fetching>().folder(acct_imap(), path)
    })
}

/// The folder row whose path is `path`: from the row that names it to the next row.
fn row<'a>(page: &'a str, path: &str, _name: &str) -> &'a str {
    let start = page
        .find(&format!("data-place=\"{path}\""))
        .unwrap_or_else(|| panic!("no row for {path}: {page}"));
    let tail = &page[start + 1..];
    let end = tail
        .find("data-place=\"")
        .map_or(page.len(), |at| start + 1 + at);
    &page[start..end]
}

/// The count a row's badge shows: quire's badge label.
fn count(row: &str) -> Option<&str> {
    const LABEL: &str = "class=\"ds-badge-label\">";
    let text = &row[row.find(LABEL)? + LABEL.len()..];
    Some(&text[..text.find('<')?])
}

/// The list pane: from the list bar to the end.
fn list(page: &str) -> &str {
    &page[page.find("class=\"list-head\"").unwrap()..]
}

#[tokio::test]
async fn choosing_a_folder_lists_what_the_server_holds_there_and_its_badge_counts_it() {
    let (store, _dir, _) = store();
    let (fetcher, _) = counting(Ok(()));
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
        listed.contains("data-style=\"title\">2026<"),
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
    let current = page[..name].rfind("aria-selected=").unwrap();
    assert!(
        page[current..].starts_with("aria-selected=\"true\""),
        "the row is not marked as chosen: {}",
        &page[current..name]
    );
    // One account in view: the title names no address.
    assert!(!listed.contains("me@nowhere.example"), "{listed}");
}

#[tokio::test]
async fn opening_fetches_once_per_choice_and_not_again_within_the_minute() {
    let (store, _dir, _) = store();
    let (fetcher, calls) = counting(Ok(()));
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
    // Where the folder stands is the fetching's, not a line in the list bar: opening it no
    // longer writes the status every account shares.
    assert!(
        matches!(
            folder_state(&mut dom, PROJECTS),
            FolderFetch::Fetched { .. }
        ),
        "{:?}",
        folder_state(&mut dom, PROJECTS)
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
        seen.one("data-hint", "Show the folders you do not follow"),
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
async fn a_fetch_that_fails_is_the_folders_to_say() {
    let (store, _dir, _) = store();
    let (fetcher, calls) = counting(Err("the server has no such folder any more"));
    let (mut dom, seen) = window(store, fetcher);
    click(&mut dom, seen.folder(RECEIPTS));
    settle(&mut dom).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        folder_state(&mut dom, RECEIPTS),
        FolderFetch::Refused(format!(
            "{RECEIPTS} was not fetched: the server has no such folder any more"
        ))
    );
    // And it does not reach the list bar as a failure of every account's sync.
    let page = dioxus_ssr::render(&dom);
    assert!(
        !list(&page).contains("class=\"ds-label status bad\""),
        "{}",
        list(&page)
    );
    // What was already held is still listed.
    assert!(
        list(&page).contains("Your receipt for September"),
        "{}",
        list(&page)
    );
}

#[test]
fn an_op_in_a_folder_place_keeps_the_row_while_its_mail_is_still_there() {
    let (store, _dir, threads) = store();
    let places = places_with(&[], &super::folder_places(&store), &[]);
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
    let mut gone = batch(acct_imap(), PROJECTS);
    gone.gone.push(remote(PROJECTS, 1));
    store.ingest(acct_imap(), gone).unwrap();
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
    let account = rows.last().unwrap().id.clone();
    let caps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        ..crate::ui::fixtures::gmail_caps()
    };
    built
        .store
        .put_caps(account.clone(), &caps, Utc::now())
        .unwrap();
    let listed = |path: &str| Folder {
        account: account.clone(),
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
    built.store.put_folders(account.clone(), folders).unwrap();
    for (uid, subject) in [
        (41, "Q3 plan draft, with the open questions"),
        (42, "Budget for the Q3 plan"),
        (43, "Offsite venue: two options"),
    ] {
        deliver(&built.store, account.clone(), PROJECTS, uid, subject);
    }
    deliver(
        &built.store,
        account,
        RECEIPTS,
        44,
        "Your receipt for September",
    );
    let space = crate::ui::space::load(&built.dirs.config).current_space();
    let (fetcher, _) = counting(Ok(()));
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs)
        .with_root_context(fetcher);
    let seen = rebuild_into(&mut dom);
    click(&mut dom, seen.folder(PROJECTS));
    settle(&mut dom).await;
    let body = dioxus_ssr::render(&dom);
    for (suffix, scheme) in [("", Scheme::Light), ("-dark", Scheme::Dark)] {
        let framed = crate::ui::fixtures::framed(&body, scheme, &space.look);
        crate::ui::fixtures::write_page(
            &format!("folder-place{suffix}"),
            &crate::ui::fixtures::page(&framed, ""),
        );
    }
}
