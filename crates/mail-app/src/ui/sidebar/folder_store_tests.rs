//! The Folders section against a real store: a round trip through the window's own function,
//! and the section as the frame draws it.

use super::super::app::App;
use super::folder_act::{load, perform, refused};
use super::folder_tests::{IMAP, POP, folder, shape};
use super::folder_tree::{Show, arrange};
use crate::folder::Refusal;
use crate::ui::fixtures::{chord, click, dispatching, empty, rebuild_into, right_click, type_into};
use crate::ui::ops::take_back;
use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::ElementId;
use ds::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

fn configure(store: &SqliteStore, id: AccountId, preset: presets::Preset) {
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                id.to_string(),
                preset.plan.address.clone(),
                serde_json::to_string(&preset.plan).unwrap(),
                Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();
    store
        .put_caps(id, &preset.expected_caps, Utc::now())
        .unwrap();
}

/// One IMAP account whose folders have been listed, nested and with a special use among them.
fn imap_store() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let (store, dir) = empty();
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    configure(
        &store,
        IMAP,
        presets::manual("me@nowhere.example", &manual, Utc::now()),
    );
    let mut sent = folder("Sent", Some('/'));
    sent.special = Some(SpecialUse::Sent);
    let mut old = folder("Old news", Some('/'));
    old.subscription = Subscription::Unsubscribed;
    store
        .put_folders(
            IMAP,
            vec![
                folder("INBOX", Some('/')),
                sent,
                folder("Projects", Some('/')),
                folder("Projects/2026", Some('/')),
                folder("收據", Some('/')),
                old,
            ],
        )
        .unwrap();
    (store, dir)
}

fn add_pop(store: &SqliteStore) {
    let manual = presets::ManualPop3 {
        pop3_host: "pop.nowhere.example".to_owned(),
        pop3_port: 995,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    configure(
        store,
        POP,
        presets::manual_pop3("you@nowhere.example", &manual, Utc::now()),
    );
}

/// What the window draws over the next moments: a picked menu item blinks, then closes, then
/// acts, on quire's clock.
async fn later(dom: &mut VirtualDom) -> crate::ui::fixtures::Seen {
    let mut seen = crate::ui::fixtures::Seen::default();
    for _ in 0..10 {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(60), dom.wait_for_work()).await;
        let mut more = crate::ui::fixtures::Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    seen
}

fn drawn(store: &SqliteStore) -> String {
    arrange(&load(store, &[IMAP]), Show::Followed)
        .map(|section| shape(&section.trees[0].nodes))
        .unwrap_or_default()
}

#[test]
fn made_renamed_and_deleted_through_the_window_and_each_undone() {
    let (store, _dir) = imap_store();
    let before = drawn(&store);
    assert_eq!(before, "Projects (2026), 收據");
    let queued = |store: &SqliteStore| store.outbox_due(IMAP, Utc::now()).unwrap().len();

    let create = FolderWork::Create {
        path: "Projects/Receipts".to_owned(),
    };
    let made = perform(&store, IMAP, create, Some('/')).unwrap();
    assert_eq!(made.said, "Folder “Receipts” made");
    assert_eq!(drawn(&store), "Projects (2026, Receipts), 收據");

    let rename = FolderWork::Rename {
        from: "Projects/Receipts".to_owned(),
        to: "Projects/Bills".to_owned(),
    };
    let moved = perform(&store, IMAP, rename, Some('/')).unwrap();
    assert_eq!(moved.said, "“Receipts” renamed to “Bills”");
    assert_eq!(drawn(&store), "Projects (2026, Bills), 收據");

    let delete = FolderWork::Delete {
        path: "Projects/Bills".to_owned(),
        non_empty: NonEmpty::Refuse,
    };
    let gone = perform(&store, IMAP, delete, Some('/')).unwrap();
    assert_eq!(gone.said, "Folder “Bills” deleted");
    assert_eq!(drawn(&store), "Projects (2026), 收據");
    assert_eq!(queued(&store), 3, "each was queued for the server");

    // Undone newest first, as Ctrl Z does, and each tells the server the reverse.
    assert!(take_back(&store, gone.undo.as_ref().unwrap()));
    assert_eq!(drawn(&store), "Projects (2026, Bills), 收據");
    assert!(take_back(&store, moved.undo.as_ref().unwrap()));
    assert_eq!(drawn(&store), "Projects (2026, Receipts), 收據");
    assert!(take_back(&store, made.undo.as_ref().unwrap()));
    assert_eq!(drawn(&store), before);
    let sent: Vec<ProtoOp> = store
        .outbox_due(IMAP, Utc::now())
        .unwrap()
        .into_iter()
        .map(|entry| entry.op)
        .collect();
    assert_eq!(
        sent[3..],
        [
            ProtoOp::Folder(FolderWork::Create {
                path: "Projects/Bills".to_owned()
            }),
            ProtoOp::Folder(FolderWork::Rename {
                from: "Projects/Bills".to_owned(),
                to: "Projects/Receipts".to_owned()
            }),
            ProtoOp::Folder(FolderWork::Delete {
                path: "Projects/Receipts".to_owned(),
                non_empty: NonEmpty::Refuse
            }),
        ]
    );
}

#[test]
fn following_is_undone_too_and_a_refusal_changes_nothing() {
    let (store, _dir) = imap_store();
    let stop = FolderWork::Subscribe {
        path: "收據".to_owned(),
        subscription: Subscription::Unsubscribed,
    };
    let done = perform(&store, IMAP, stop, Some('/')).unwrap();
    assert_eq!(done.said, "Stopped following “收據”");
    assert_eq!(drawn(&store), "Projects (2026)");
    assert!(take_back(&store, done.undo.as_ref().unwrap()));
    assert_eq!(drawn(&store), "Projects (2026), 收據");

    let before = store.outbox_due(IMAP, Utc::now()).unwrap().len();
    let taken = FolderWork::Create {
        path: "Projects".to_owned(),
    };
    let refusal = perform(&store, IMAP, taken, Some('/')).unwrap_err();
    assert_eq!(
        refused(&refusal),
        "There is already a folder called “Projects”."
    );
    let sent = FolderWork::Rename {
        from: "Sent".to_owned(),
        to: "Posted".to_owned(),
    };
    let refusal = perform(&store, IMAP, sent, Some('/')).unwrap_err();
    assert!(refused(&refusal).contains("Sent folder"), "{refusal}");
    assert_eq!(store.outbox_due(IMAP, Utc::now()).unwrap().len(), before);
}

#[test]
fn a_delete_that_takes_mail_is_asked_first_and_offers_no_undo() {
    let (store, _dir) = imap_store();
    let mailbox = MailboxRef {
        account: IMAP,
        path: "收據".to_owned(),
    };
    let raw = store.blobs().put(&store.connection(), b"x").unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: IMAP,
        key: MessageKey::Rfc("r1@example.test".to_owned()),
        date: Utc::now(),
        from: Address {
            name: None,
            email: "shop@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "receipt".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Read,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    store
        .ingest(
            IMAP,
            Ingest {
                mailbox: mailbox.clone(),
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: "收據".to_owned(),
                        uidvalidity: 1,
                        uid: 7,
                    },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();
    let asked = perform(
        &store,
        IMAP,
        FolderWork::Delete {
            path: "收據".to_owned(),
            non_empty: NonEmpty::Refuse,
        },
        Some('/'),
    )
    .unwrap_err();
    assert!(matches!(
        asked,
        Refusal::Folder(FolderError::NotEmpty { messages: 1, .. })
    ));
    let done = perform(
        &store,
        IMAP,
        FolderWork::Delete {
            path: "收據".to_owned(),
            non_empty: NonEmpty::Allow,
        },
        Some('/'),
    )
    .unwrap();
    assert_eq!(done.said, "Folder “收據” deleted with its mail");
    assert_eq!(done.undo, None, "a name comes back, the mail would not");
    assert_eq!(drawn(&store), "Projects (2026)");
}

// ---- the section on the frame ------------------------------------------------------------

fn frame(store: Arc<SqliteStore>) -> String {
    let mut dom = VirtualDom::new(App).with_root_context(store);
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

#[tokio::test]
async fn the_section_draws_nested_folders_and_leaves_out_places_and_unfollowed_ones() {
    let (store, _dir) = imap_store();
    let page = frame(store);
    assert!(page.contains(">Folders"), "{page}");
    for name in ["Projects", "2026", "收據"] {
        assert!(
            page.contains(&format!("aria-label=\"Actions for {name}\"")),
            "{name} is not drawn: {page}"
        );
    }
    for name in ["INBOX", "Sent", "Old news"] {
        assert!(
            !page.contains(&format!("aria-label=\"Actions for {name}\"")),
            "{name} is drawn: {page}"
        );
    }
    // 2026 is inside Projects: Projects is an outline branch, and its row is followed by 2026's.
    let projects = page.find("Actions for Projects").unwrap();
    assert!(
        page[..projects].rfind("class=\"ds-row-branch\"").is_some(),
        "Projects is not a branch: {page}"
    );
    assert!(page[projects..].contains("Actions for 2026"), "{page}");
    assert!(page.contains("Show all"), "{page}");
}

#[tokio::test]
async fn a_pop3_scope_has_no_section() {
    let (store, _dir) = empty();
    add_pop(&store);
    let page = frame(store);
    assert!(!page.contains(">Folders"), "{page}");
}

#[tokio::test]
async fn the_menu_and_the_field_are_the_shared_ones_and_styled() {
    dispatching();
    let (store, _dir) = imap_store();
    let mut dom = VirtualDom::new(App).with_root_context(store);
    let seen = rebuild_into(&mut dom);
    // The ⋯ is quire's `PopUpButton`, which opens against its own mounted element; a document
    // with no renderer never reports one, so the same menu is asked for by a right-click.
    let mut seen = right_click(&mut dom, seen.folder("Projects"));
    // quire's menu floats in the root's overlay, drawn on the render after it is asked for.
    for _ in 0..8 {
        dom.process_events();
        let mut more = crate::ui::fixtures::Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    let menu = dioxus_ssr::render(&dom);
    assert!(menu.contains("class=\"ds-menu"), "{menu}");
    for item in ["New folder inside", "Rename", "Stop following", "Delete"] {
        assert!(menu.contains(item), "{item} missing: {menu}");
    }
    // The first item is New folder inside.
    let first = seen.fixed("class", "ds-menu-item")[0];
    let picked = click(&mut dom, first);
    let seen = picked.merge(later(&mut dom).await);
    let field = seen.one("aria-placeholder", "New folder");
    type_into(&mut dom, field, "a/b");
    chord(&mut dom, "Enter", Modifiers::empty(), field);
    let naming = dioxus_ssr::render(&dom);
    // The shared field: quire's `TextField`, without its bezel.
    assert!(
        naming.contains("class=\"ds-input\" data-variant=\"plain\""),
        "{naming}"
    );
    assert!(
        naming.contains("A folder name cannot contain “/”"),
        "the refusal is not said: {naming}"
    );
    let offences = crate::ui::style::tests::markup_offences(&(menu + &naming));
    assert!(offences.is_empty(), "the markup lint: {offences:#?}");
}

/// The window with `Projects`' ⋯ menu open and its Rename picked, and the rename field.
async fn renaming_projects() -> (VirtualDom, ElementId, Arc<SqliteStore>, tempfile::TempDir) {
    let (store, dir) = imap_store();
    let mut dom = VirtualDom::new(App).with_root_context(store.clone());
    let seen = rebuild_into(&mut dom);
    let mut seen = right_click(&mut dom, seen.folder("Projects"));
    for _ in 0..8 {
        dom.process_events();
        let mut more = crate::ui::fixtures::Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    // The menu's items in order: New folder inside, then Rename.
    let rename = seen.fixed("class", "ds-menu-item")[1];
    let picked = click(&mut dom, rename);
    let field = picked
        .merge(later(&mut dom).await)
        .one("aria-placeholder", "Folder name");
    (dom, field, store, dir)
}

/// The rename field: quire's `TextField` in the row's words, in place of the name.
fn editing_markup(page: &str) -> Option<&str> {
    let at = page.find("aria-label=\"Folder name\"")?;
    let start = page[..at].rfind("<input")?;
    let end = page[at..].find("/>").map_or(page.len(), |end| at + end);
    Some(&page[start..end])
}

#[tokio::test]
async fn a_rename_is_written_in_the_name_s_place_and_enter_makes_it() {
    dispatching();
    let (mut dom, field, store, _dir) = renaming_projects().await;
    let page = dioxus_ssr::render(&dom);
    let slot = editing_markup(&page).unwrap_or_else(|| panic!("no editing slot:\n{page}"));
    assert!(
        slot.contains("class=\"ds-input\" data-variant=\"plain\"")
            && slot.contains("value=\"Projects\""),
        "{slot}"
    );
    type_into(&mut dom, field, "Plans");
    chord(&mut dom, "Enter", Modifiers::empty(), field);
    let page = dioxus_ssr::render(&dom);
    assert!(editing_markup(&page).is_none(), "Enter left the field");
    let paths: Vec<String> = store
        .folders(IMAP)
        .unwrap()
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert!(paths.contains(&"Plans".to_owned()), "{paths:?}");
    assert!(!paths.contains(&"Projects".to_owned()), "{paths:?}");
}

#[tokio::test]
async fn escape_takes_the_rename_away_and_keeps_the_name() {
    dispatching();
    let (mut dom, field, store, _dir) = renaming_projects().await;
    chord(&mut dom, "Escape", Modifiers::empty(), field);
    let page = dioxus_ssr::render(&dom);
    assert!(editing_markup(&page).is_none(), "Escape left the field");
    assert!(page.contains(">Projects</b>"), "{page}");
    let paths: Vec<String> = store
        .folders(IMAP)
        .unwrap()
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert!(paths.contains(&"Projects".to_owned()), "{paths:?}");
}

#[tokio::test]
async fn several_accounts_are_each_named_over_their_folders() {
    let (store, _dir) = imap_store();
    let other = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f3"));
    let manual = presets::Manual {
        imap_host: "imap.elsewhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.elsewhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    configure(
        &store,
        other,
        presets::manual("me@elsewhere.example", &manual, Utc::now()),
    );
    let mut lists = folder("Lists", Some('.'));
    lists.account = other;
    let mut inbox = folder("INBOX", Some('.'));
    inbox.account = other;
    store.put_folders(other, vec![inbox, lists]).unwrap();
    let page = frame(store);
    assert!(page.contains("me@elsewhere.example"), "{page}");
    let offences = crate::ui::style::tests::markup_offences(&page);
    assert!(offences.is_empty(), "the markup lint: {offences:#?}");
}

/// Writes `target/folders.html` and `folders-menu.html`, each with a `-dark` twin: the Work
/// Space's frame with a Folders section, and the same with a folder's menu open.
#[tokio::test]
#[ignore = "writes target/folders*.html for a human or a headless browser to look at"]
async fn render_the_folders_to_a_file() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let account = crate::ui::data::account_rows(&built.store)[0].id;
    let listed = |path: &str| Folder {
        account,
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    };
    let mut holder = listed("[Gmail]");
    holder.holds = Holds::FoldersOnly;
    let mut sent = listed("[Gmail]/Sent Mail");
    sent.special = Some(SpecialUse::Sent);
    let mut old = listed("Old newsletters");
    old.subscription = Subscription::Unsubscribed;
    let folders = vec![
        listed("INBOX"),
        holder,
        sent,
        listed("Projects"),
        listed("Projects/2026"),
        listed("Projects/2026/Q3"),
        listed("Projects/Archive 2019"),
        listed("收據"),
        listed("旅行/京都"),
        old,
    ];
    built.store.put_folders(account, folders).unwrap();
    let space = crate::space::load(&built.dirs.config).current_space();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs);
    let seen = rebuild_into(&mut dom);
    let closed = dioxus_ssr::render(&dom);
    right_click(&mut dom, seen.folder("Projects/2026"));
    let open = dioxus_ssr::render(&dom);
    for (name, body) in [("folders", &closed), ("folders-menu", &open)] {
        for (suffix, scheme) in [("", Scheme::Light), ("-dark", Scheme::Dark)] {
            let framed = crate::ui::fixtures::framed(body, scheme, &space.look);
            crate::ui::fixtures::write_page(
                &format!("{name}{suffix}"),
                &crate::ui::fixtures::page(&framed, ""),
            );
        }
    }
}
