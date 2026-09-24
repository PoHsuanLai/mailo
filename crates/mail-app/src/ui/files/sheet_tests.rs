//! The sheets as drawn, every class they use styled, a file of each to look at, and local folders
//! as the frame draws them: named so, and with nothing to sync.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

use super::work::{Dest, Looked, import_now, look};
use super::{FilesSheet, Phase, Progress};
use crate::ui::fixtures::{click, dispatching, rebuild_into};
use crate::view::{FileSheet, Shell};

/// The sheet `open` names, alone.
#[component]
fn Sheet(open: FileSheet) -> Element {
    let shell = use_signal(|| Shell {
        files: Some(open.clone()),
        ..Shell::default()
    });
    let revision = use_signal(|| 0u64);
    rsx! { FilesSheet { shell, revision } }
}

/// The sheet over `store`, suggesting and starting in `saves` rather than the downloads directory.
fn sheet(store: &Arc<SqliteStore>, saves: &Path, open: FileSheet) -> VirtualDom {
    VirtualDom::new_with_props(Sheet, SheetProps { open })
        .with_root_context(store.clone())
        .with_root_context(super::SaveDir(saves.to_owned()))
}

/// The progress line in every phase a run passes through.
#[component]
fn Phases() -> Element {
    rsx! {
        for phase in [
            Phase::Running { done: 500, of: 1204 },
            Phase::Finished("1204 message(s) read; 1204 kept in local folders; 0 already there".to_owned()),
            Phase::Failed("There is nothing at ~/gone.mbox.".to_owned()),
        ] {
            div { class: "files-foot", Progress { phase, verb: "Importing" } }
        }
    }
}

/// A Maildir of two under `under`.
fn a_maildir(under: &Path) -> PathBuf {
    let root = under.join("Takeout");
    for sub in ["cur", "new", "tmp"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    for (name, subject) in [
        ("1699363251.M1.host:2,S", "Lunch"),
        ("1699363252.M2.host", "Tea"),
    ] {
        std::fs::write(
            root.join("cur").join(name),
            format!(
                "Message-ID: <{subject}@example.test>\nFrom: ada@example.test\n\
                 Subject: {subject}\nDate: Tue, 07 Nov 2023 13:20:51 +0000\n\nhello\n"
            ),
        )
        .unwrap();
    }
    root
}

/// Keep `root`'s mail in local folders, as the sheet's Import does.
fn import_locally(store: &SqliteStore, root: &Path) -> AccountId {
    let Looked::Mail { source, .. } = look(root) else {
        panic!("{} is not mail", root.display());
    };
    import_now(
        store,
        &source,
        &Dest::Local,
        chrono::Utc::now(),
        &mut |_| {},
    )
    .unwrap();
    crate::account::local(store, chrono::Utc::now()).unwrap()
}

#[tokio::test]
async fn the_import_sheet_says_what_is_at_the_path_and_where_it_goes() {
    dispatching();
    let (store, dir) = crate::ui::fixtures::seeded();
    let root = a_maildir(dir.path());
    let mut dom = sheet(
        &store,
        dir.path(),
        FileSheet::Import {
            path: root.display().to_string(),
        },
    );
    let seen = rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Maildir, 2 messages"), "{page}");
    assert!(page.contains("Local folders"), "{page}");
    click(
        &mut dom,
        seen.one("aria-label", "Import into: Local folders"),
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("This computer"),
        "the menu did not open: {page}"
    );
}

#[tokio::test]
async fn the_import_sheet_refuses_in_words() {
    dispatching();
    let (store, dir) = crate::ui::fixtures::seeded();
    let gone = dir.path().join("gone.mbox");
    let mut dom = sheet(
        &store,
        dir.path(),
        FileSheet::Import {
            path: gone.display().to_string(),
        },
    );
    rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(&format!("There is nothing at {}.", gone.display())),
        "{page}"
    );
    assert!(
        page.contains("disabled"),
        "Import is offered for nothing: {page}"
    );
}

/// Both sheets, the destination menu open, and every phase of a run.
fn every_state(store: &Arc<SqliteStore>, root: &Path) -> String {
    let mut import = sheet(
        store,
        root,
        FileSheet::Import {
            path: root.display().to_string(),
        },
    );
    let seen = rebuild_into(&mut import);
    click(
        &mut import,
        seen.one("aria-label", "Import into: Local folders"),
    );
    let mut export = sheet(
        store,
        root,
        FileSheet::Export {
            query: "inbox".to_owned(),
        },
    );
    export.rebuild_in_place();
    let mut phases = VirtualDom::new(Phases);
    phases.rebuild_in_place();
    dioxus_ssr::render(&import) + &dioxus_ssr::render(&export) + &dioxus_ssr::render(&phases)
}

#[tokio::test]
async fn every_class_the_mail_file_sheets_draw_is_styled() {
    dispatching();
    let (store, dir) = crate::ui::fixtures::seeded();
    let markup = every_state(&store, &a_maildir(dir.path()));
    for class in ["files-dest", "files-bar", "files-bad", "seg"] {
        assert!(markup.contains(class), "{class} was not drawn");
    }
    let missing = crate::ui::style::tests::unstyled_classes(&markup, crate::ui::style::STYLE);
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

/// Whether the list bar offers a sync.
fn syncs(page: &str) -> bool {
    page.contains("aria-label=\"Sync now\"")
}

#[tokio::test]
async fn local_folders_are_named_so_and_have_nothing_to_sync() {
    dispatching();
    let (store, dir) = crate::ui::fixtures::seeded();
    import_locally(&store, &a_maildir(dir.path()));
    // One Space over every account, so both tiles are drawn side by side.
    let everything = crate::space::Spaces {
        spaces: vec![crate::space::Space::default()],
        current: 0,
        recall: std::collections::BTreeMap::new(),
    };
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(store.clone())
        .with_root_context(everything);
    let seen = rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        syncs(&page),
        "an IMAP account is in view, so there is a sync: {page}"
    );
    assert!(
        !page.contains("local folders"),
        "the address leaked: {page}"
    );

    click(&mut dom, seen.one("aria-label", "Local folders"));
    let page = dioxus_ssr::render(&dom);
    assert!(!syncs(&page), "local folders offered a sync");
    assert!(
        page.contains("<span class=\"mono\">Local folders</span>"),
        "the header does not say Local folders: {page}"
    );

    // Local folders alone: nothing in the window syncs.
    let (only, dir) = crate::ui::fixtures::empty();
    import_locally(&only, &a_maildir(dir.path()));
    let page = crate::ui::fixtures::markup(only);
    assert!(!syncs(&page), "local folders alone offered a sync: {page}");
}

#[test]
fn local_folders_send_nothing_and_tell_no_server() {
    let (store, dir) = crate::ui::fixtures::seeded();
    let sending_before = crate::compose::sending_accounts(&store);
    let local = import_locally(&store, &a_maildir(dir.path()));
    assert_eq!(crate::compose::sending_accounts(&store), sending_before);
    assert!(
        crate::compose::account_for(&store, Some(presets::LOCAL_FOLDERS)).is_err(),
        "local folders were offered as a From"
    );

    let thread = store
        .threads(
            &Query {
                filter: Filter::Account(local),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .items[0]
        .clone();
    assert_eq!(thread.star, Star::Unstarred);
    assert!(crate::ui::ops::apply_op(&store, thread.id, OpKind::Star));
    let starred = store.thread(thread.id).unwrap().summary.star;
    assert_eq!(starred, Star::Starred, "starring local mail works");
    let far = chrono::Utc::now() + chrono::TimeDelta::days(3650);
    assert!(
        store.outbox_due(local, far).unwrap().is_empty(),
        "local folders' outbox is never drained, so nothing is put in it"
    );
}

/// `extra` as the first child of `.app`, where the window mounts its overlays.
fn inject(page: &str, extra: &str) -> String {
    let at = page.find("class=\"app").unwrap_or(0);
    let close = page[at..].find('>').map_or(page.len(), |rel| at + rel + 1);
    format!("{}{extra}{}", &page[..close], &page[close..])
}

#[tokio::test]
#[ignore = "writes target/import.html, target/export.html and their -dark twins to look at"]
async fn render_the_mail_file_sheets_to_files() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let root = a_maildir(built.root.path());
    let saves = built.root.path().join("Downloads");
    std::fs::create_dir_all(&saves).unwrap();
    let mut frame = VirtualDom::new(crate::ui::app::App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    frame.rebuild_in_place();
    let backdrop = dioxus_ssr::render(&frame);

    let mut import = sheet(
        &built.store,
        &saves,
        FileSheet::Import {
            path: root.display().to_string(),
        },
    );
    let seen = rebuild_into(&mut import);
    click(
        &mut import,
        seen.one("aria-label", "Import into: Local folders"),
    );
    crate::ui::fixtures::dump("import", &inject(&backdrop, &dioxus_ssr::render(&import)));

    let mut export = sheet(
        &built.store,
        &saves,
        FileSheet::Export {
            query: "inbox".to_owned(),
        },
    );
    export.rebuild_in_place();
    // The count is worked out off the thread that draws; let it land.
    for _ in 0..10 {
        let quiet = std::time::Duration::from_millis(200);
        if tokio::time::timeout(quiet, export.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        export.render_immediate(&mut NoOpMutations);
    }
    crate::ui::fixtures::dump("export", &inject(&backdrop, &dioxus_ssr::render(&export)));
}
