//! The sheets as drawn, every class they use styled, a file of each to look at, and local folders
//! as the frame draws them: named so, and with nothing to sync.

use ds::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use porter_core::AccountId;

use super::work::{Dest, Looked, import_now, look};
use super::{FilesSheet, Phase, Report};
use crate::ui::fixtures::{Seen, click, dispatching, drain_seen, rebuild_into};
use mail_ui_kit::view::{FileSheet, Shell};

/// The first render, then the ones after it, keeping every attribute they set: a quire sheet is
/// drawn a frame after the one that asked for it, on quire's clock.
async fn landed(dom: &mut VirtualDom) -> Seen {
    let mut seen = rebuild_into(dom);
    for _ in 0..40 {
        let quiet = std::time::Duration::from_millis(150);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut seen);
    }
    seen.merge(drain_seen(dom))
}

/// The sheet `open` names, alone.
#[component]
fn Sheet(open: FileSheet) -> Element {
    let shell = use_signal(|| Shell {
        files: Some(open.clone()),
        ..Shell::default()
    });
    // Inside a quire root, as the window has it: the sheet's menu floats in its overlay.
    rsx! {
        Ds {
            appearance: Appearance::default(),
            material: Material::Window,
            stylesheet: ds::assembly::ds::Inject::Host,
            FilesSheet { shell }
        }
    }
}

/// The sheet over `store`, suggesting and starting in `saves` rather than the downloads directory.
fn sheet(store: &Arc<SqliteStore>, saves: &Path, open: FileSheet) -> VirtualDom {
    VirtualDom::new_with_props(Sheet, SheetProps { open })
        .with_root_context(crate::ui::fixtures::client(store))
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
            div { class: "sheet-actions", Report { phase, verb: "Importing" } }
        }
    }
}

/// A Maildir of two under `under`.
fn a_maildir(under: &Path) -> PathBuf {
    let root = under.join("Takeout");
    for sub in ["cur", "new", "tmp"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let info = mail_archive::maildir::INFO;
    for (name, subject) in [
        (format!("1699363251.M1.host{info}2,S"), "Lunch"),
        ("1699363252.M2.host".to_owned(), "Tea"),
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
fn import_locally(store: &Arc<SqliteStore>, root: &Path) -> AccountId {
    let Looked::Mail { source, .. } = look(root) else {
        panic!("{} is not mail", root.display());
    };
    import_now(
        &crate::ui::fixtures::client(store),
        &source,
        &Dest::Local,
        chrono::Utc::now(),
        &mut |_| {},
    )
    .unwrap();
    mail_core::account::local(store, chrono::Utc::now()).unwrap()
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
    let seen = landed(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Maildir, 2 messages"), "{page}");
    assert!(page.contains("Local folders"), "{page}");
    click(
        &mut dom,
        seen.one("aria-label", "Import into: Local folders"),
    );
    crate::ui::fixtures::drain(&mut dom);
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
    landed(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(&format!("Nothing at {}.", gone.display())),
        "{page}"
    );
    // The sheet's Import button is the one that answers Return.
    let at = page
        .find("data-answers=\"return\"")
        .unwrap_or_else(|| panic!("no Import button: {page}"));
    let start = page[..at].rfind('<').unwrap();
    let end = at + page[at..].find('>').unwrap();
    assert!(
        page[start..end].contains("aria-disabled=\"true\""),
        "Import is offered for nothing: {page}"
    );
}

/// Both sheets, the destination menu open, and every phase of a run.
async fn every_state(store: &Arc<SqliteStore>, root: &Path) -> String {
    let mut import = sheet(
        store,
        root,
        FileSheet::Import {
            path: root.display().to_string(),
        },
    );
    let seen = landed(&mut import).await;
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

/// Every state the sheets draw, for the style lint in `style::tests`.
pub(in crate::ui) async fn drawn_states() -> String {
    dispatching();
    let (store, dir) = crate::ui::fixtures::seeded();
    let markup = every_state(&store, &a_maildir(dir.path())).await;
    for class in ["ds-progress", "role=\"alert\""] {
        assert!(markup.contains(class), "{class} was not drawn");
    }
    markup
}

/// Whether the window offers a sync: the sync mark at the sidebar's foot.
fn syncs(page: &str) -> bool {
    page.contains(&format!(
        "aria-label=\"{}\"",
        mail_ui_model::copy::aria::SYNC_NOW
    ))
}

#[tokio::test]
async fn local_folders_leak_no_address_and_have_nothing_to_sync() {
    dispatching();
    let (store, dir) = crate::ui::fixtures::seeded();
    let local = import_locally(&store, &a_maildir(dir.path()));
    // One Space showing every account, the IMAP one among them.
    let known = mail_ui_kit::data::known_accounts(&crate::ui::fixtures::client(&store))
        .expect("the accounts");
    let everything = mail_ui_kit::space::built(
        vec![(
            "Mail".to_owned(),
            ds::prelude::SpaceLook::default(),
            mail_ui_kit::space::Mail::of(known),
        )],
        0,
    );
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(crate::ui::fixtures::client(&store))
        .with_root_context(everything);
    landed(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        syncs(&page),
        "an IMAP account is in view, so there is a sync: {page}"
    );
    assert!(
        !page.contains("local folders"),
        "the address leaked: {page}"
    );

    // A Space over the local folders alone, on screen; the IMAP account is in no Space.
    let alone = mail_ui_kit::space::first_run(std::slice::from_ref(&local));
    let mut dom = VirtualDom::new(crate::ui::app::App)
        .with_root_context(crate::ui::fixtures::client(&store))
        .with_root_context(alone);
    landed(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    // One Space draws no name over the sidebar, so nothing here says whose mail it is.
    assert!(!syncs(&page), "local folders offered a sync");

    // Local folders alone: nothing in the window syncs.
    let (only, dir) = crate::ui::fixtures::empty();
    import_locally(&only, &a_maildir(dir.path()));
    let page = crate::ui::fixtures::markup(only);
    assert!(!syncs(&page), "local folders alone offered a sync: {page}");
}

#[test]
fn local_folders_send_nothing_and_tell_no_server() {
    let (store, dir) = crate::ui::fixtures::seeded();
    let sending_before = mail_core::compose::sending_accounts(&store);
    let local = import_locally(&store, &a_maildir(dir.path()));
    assert_eq!(mail_core::compose::sending_accounts(&store), sending_before);
    assert!(
        mail_core::compose::account_for(&store, Some(presets::LOCAL_FOLDERS)).is_err(),
        "local folders were offered as a From"
    );

    let thread = store
        .threads(
            &Query {
                filter: Filter::Account(local.clone()),
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
    assert!(mail_ui_kit::ops::apply_op(
        &crate::ui::fixtures::client(&store),
        thread.id,
        OpKind::Star
    ));
    let starred = store.thread(thread.id).unwrap().summary.star;
    assert_eq!(starred, Star::Starred, "starring local mail works");
    let far = chrono::Utc::now() + chrono::TimeDelta::days(3650);
    assert!(
        store.outbox_due(local, far).unwrap().is_empty(),
        "local folders' outbox is never drained, so nothing is put in it"
    );
}
