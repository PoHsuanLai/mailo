//! Motion in the running window: a row leaves after the store has already moved, an undo puts
//! the thread back exactly, and a drag onto a place is the same op as the button.

use super::drag::{drop_op, view_kind};
use crate::ui::app::App;
use crate::ui::fixtures::{
    FakePointer, INSIDE_THE_SHELL, Seen, animation_end, chord, click, dispatching, pointer,
    rebuild_into, work,
};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

const DANA: &str = "Re: UIDL stability across a UIDVALIDITY change";
/// The row under Dana's in the Inbox: the next newest.
const RELEASE: &str = "0.9 cut on Thursday: what is still open";

struct Mounted {
    dom: VirtualDom,
    store: Arc<SqliteStore>,
    dana: ThreadId,
    seen: Seen,
    _root: tempfile::TempDir,
}

fn mounted() -> Mounted {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    Mounted {
        dom,
        store: built.store,
        dana: built.dana,
        seen,
        _root: built.root,
    }
}

/// Let the off-thread list query land and redraw, as the window would between frames.
async fn settle(dom: &mut VirtualDom) {
    for _ in 0..8 {
        if tokio::time::timeout(std::time::Duration::from_millis(40), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut NoOpMutations);
    }
}

fn changes(store: &SqliteStore) -> i64 {
    store
        .connection()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap_or(0)
}

fn mailboxes(store: &SqliteStore, thread: ThreadId) -> MailboxSet {
    store.thread(thread).unwrap().summary.mailboxes
}

/// The `<li>` of the row whose subject is `subject`, as rendered.
fn row_markup(page: &str, subject: &str) -> Option<String> {
    let label = format!("aria-label=\"Open {subject}\"");
    let at = page.find(&label)?;
    let start = page[..at].rfind("<li")?;
    let end = page[at..].find("</li>").map_or(page.len(), |n| at + n);
    Some(page[start..end].to_owned())
}

/// Dana's row's Archive button. The strip draws one per row in list order, and Dana's is the
/// newest row in the Inbox.
fn dana_archive(seen: &Seen) -> ElementId {
    seen.all("aria-label", "Archive")[0]
}

fn at(x: f64, y: f64) -> FakePointer {
    FakePointer {
        client: (x, y),
        offset: (12.0, 10.0),
        held: true,
    }
}

#[tokio::test]
async fn archiving_moves_the_store_at_once_and_the_row_leaves_on_its_animationend() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    let row = seen.one("aria-label", &format!("Open {DANA}"));
    assert!(mailboxes(&store, dana).contains(MailboxRole::Inbox));

    click(&mut dom, dana_archive(&seen));
    settle(&mut dom).await;

    assert!(
        !mailboxes(&store, dana).contains(MailboxRole::Inbox),
        "the store did not have the archive before the row's animation ended"
    );
    let page = dioxus_ssr::render(&dom);
    let going = row_markup(&page, DANA).expect("the row is still drawn while it leaves");
    assert!(
        going.contains("class=\"row going\"") && going.contains("data-op=\"archive\""),
        "the leaving row is not going[data-op=archive]:\n{going}"
    );

    animation_end(&mut dom, row, "fold");
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        row_markup(&page, DANA).is_none(),
        "the row was still drawn after its exit ended"
    );
    let next = row_markup(&page, RELEASE).expect("the next row is there");
    assert!(
        next.contains("class=\"row healing\""),
        "the row under the gap does not heal into it:\n{next}"
    );
}

#[tokio::test]
async fn undo_restores_the_mailboxes_exactly() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    let before = mailboxes(&store, dana);

    let mut seen_after = click(&mut dom, dana_archive(&seen));
    assert_ne!(mailboxes(&store, dana), before, "the archive did nothing");
    // The toast is quire's, and its host lays it out a frame after the push: draw until it has.
    for _ in 0..8 {
        if seen_after.get("data-armed", "disarmed").is_some()
            || tokio::time::timeout(std::time::Duration::from_millis(100), dom.wait_for_work())
                .await
                .is_err()
        {
            break;
        }
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen_after = seen_after.merge(more);
    }
    // The pull tab: the one element that says whether it is armed.
    let undo = seen_after.one("data-armed", "disarmed");
    click(&mut dom, undo);

    assert_eq!(
        mailboxes(&store, dana),
        before,
        "undo did not put the thread back where it was"
    );
}

#[tokio::test]
async fn ctrl_z_undoes_the_same_way() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    let before = mailboxes(&store, dana);

    click(&mut dom, dana_archive(&seen));
    assert_ne!(mailboxes(&store, dana), before, "the archive did nothing");
    chord(
        &mut dom,
        "z",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );

    assert_eq!(mailboxes(&store, dana), before, "Ctrl Z did not undo");
}

#[tokio::test]
async fn dragging_a_row_onto_archive_archives_it() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    let row = seen.one("aria-label", &format!("Open {DANA}"));
    let archive = seen.one("data-place", "Archive");

    pointer(&mut dom, "pointerdown", row, at(420.0, 120.0));
    let moved = pointer(&mut dom, "pointermove", row, at(300.0, 160.0));
    let _ = moved;
    pointer(&mut dom, "pointermove", row, at(120.0, 300.0));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("class=\"ds-drag-ghost\""),
        "no ghost follows the pointer"
    );
    pointer(&mut dom, "pointerenter", archive, at(120.0, 300.0));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("is-drop-target"),
        "Archive did not light up as the target"
    );
    pointer(&mut dom, "pointerup", archive, at(120.0, 300.0));

    assert!(
        !mailboxes(&store, dana).contains(MailboxRole::Inbox),
        "dropping on Archive did not archive"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("class=\"item gulp\""),
        "the place that received the row did not gulp"
    );
}

#[tokio::test]
async fn dropping_onto_a_saved_search_writes_nothing() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    let row = seen.one("aria-label", &format!("Open {DANA}"));
    let starred = seen.one("data-place", "Starred");
    let before = changes(&store);

    pointer(&mut dom, "pointerdown", row, at(420.0, 120.0));
    pointer(&mut dom, "pointermove", row, at(120.0, 200.0));
    pointer(&mut dom, "pointerenter", starred, at(120.0, 200.0));
    pointer(&mut dom, "pointerup", starred, at(120.0, 200.0));

    assert_eq!(
        changes(&store),
        before,
        "a drop on a saved search wrote to the store"
    );
    assert!(mailboxes(&store, dana).contains(MailboxRole::Inbox));
}

#[tokio::test]
async fn escape_drops_nothing() {
    let Mounted {
        mut dom,
        store,
        seen,
        ..
    } = mounted();
    let row = seen.one("aria-label", &format!("Open {DANA}"));
    let archive = seen.one("data-place", "Archive");
    let before = changes(&store);

    pointer(&mut dom, "pointerdown", row, at(420.0, 120.0));
    pointer(&mut dom, "pointermove", row, at(120.0, 300.0));
    pointer(&mut dom, "pointerenter", archive, at(120.0, 300.0));
    chord(
        &mut dom,
        "Escape",
        Modifiers::empty(),
        ElementId(INSIDE_THE_SHELL as usize),
    );
    pointer(&mut dom, "pointerup", archive, at(120.0, 300.0));

    assert_eq!(changes(&store), before, "Esc did not cancel the drag");
}

#[test]
fn what_each_place_accepts() {
    let places = crate::view::default_places();
    let label = LabelId::generate();
    let cases: Vec<(&str, Option<Op>)> = vec![
        ("Inbox", Some(Op::Restore)),
        ("Archive", Some(Op::Archive)),
        ("Trash", Some(Op::Trash)),
        ("Spam", Some(Op::Spam)),
        // Saved searches and places nothing is filed into refuse.
        ("Starred", None),
        ("Snoozed", None),
        ("Pinned", None),
        ("Sent", None),
        ("Drafts", None),
    ];
    for (name, want) in cases {
        let place = places
            .iter()
            .find(|place| place.name == name)
            .unwrap_or_else(|| panic!("no place {name}"));
        assert_eq!(drop_op(&view_kind(place)), want, "{name}");
    }
    let labelled = crate::view::Place {
        name: "spec".to_owned(),
        source: crate::view::Source::Mail(Filter::HasLabel(label)),
        unread: None,
    };
    assert_eq!(
        drop_op(&view_kind(&labelled)),
        Some(Op::Label(label, Membership::In))
    );
}
