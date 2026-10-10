//! Motion in the running window: a row leaves after the store has already moved, an undo puts
//! the thread back exactly, and a drag onto a place is the same op as the button.

use super::drag::{drop_op, view_kind};
use crate::ui::app::App;
use crate::ui::fixtures::{
    FakePointer, INSIDE_THE_SHELL, Seen, chord, click, dispatching, open_row_menu, pick_until,
    pointer, rebuild_into, work,
};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, NoOpMutations, VirtualDom};
use ds::motion::settle::settle as anim_settle;
use ds::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
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
    mounted_with(|_, _| {})
}

/// [`mounted`], with the store changed first by `before`.
fn mounted_with(before: impl FnOnce(&SqliteStore, ThreadId)) -> Mounted {
    dispatching();
    let built = work();
    before(&built.store, built.dana);
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

/// Draw until `done`, or until `bound` has passed. A loaded runner fires the roster's wall-clock
/// sleep after the nominal settle, so a test that stops at that instant still sees the previous
/// state. Healing is brief: this returns on the render where `done` first holds, and yields
/// between renders so a document that always has work cannot spin the timer out of the slice.
async fn until(
    dom: &mut VirtualDom,
    bound: std::time::Duration,
    done: impl Fn(&str) -> bool,
) -> bool {
    let deadline = tokio::time::Instant::now() + bound;
    let frame = std::time::Duration::from_millis(16);
    loop {
        if done(&dioxus_ssr::render(dom)) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let slice = left.min(std::time::Duration::from_millis(50));
        let started = tokio::time::Instant::now();
        if tokio::time::timeout(slice, dom.wait_for_work())
            .await
            .is_ok()
        {
            dom.render_immediate(&mut NoOpMutations);
        }
        let spent = started.elapsed();
        if spent < frame {
            tokio::time::sleep(frame - spent).await;
        }
    }
}

/// How long a row's exit takes to settle at the window's motion level.
fn exit_settles() -> std::time::Duration {
    anim_settle(Anim::RowOut, MotionLevel::Standard)
}

fn changes(store: &SqliteStore) -> i64 {
    mail_store::testing::total_changes(store)
}

fn mailboxes(store: &SqliteStore, thread: ThreadId) -> MailboxSet {
    store.thread(thread).unwrap().summary.mailboxes
}

/// The list item (`div.ds-list-item`) of the row whose subject is `subject`, as rendered.
fn row_markup(page: &str, subject: &str) -> Option<String> {
    let label = format!("aria-label=\"Open {subject}\"");
    let at = page.find(&label)?;
    let start = page[..at].rfind("class=\"ds-list-item\"")?;
    let end = page[at..]
        .find("class=\"ds-list-item\"")
        .map_or(page.len(), |n| at + n);
    Some(page[start..end].to_owned())
}

/// Archive Dana's row from its menu, drawing only until the store has it: what the archive
/// started, the row's exit, is still under way.
async fn archive_dana(dom: &mut VirtualDom, seen: &Seen, store: &SqliteStore, dana: ThreadId) {
    let opened = open_row_menu(dom, seen, DANA);
    pick_until(dom, &opened, "Archive", || {
        !mailboxes(store, dana).contains(MailboxRole::Inbox)
    })
    .await;
}

fn at(x: f64, y: f64) -> FakePointer {
    FakePointer {
        client: (x, y),
        offset: (12.0, 10.0),
        held: true,
    }
}

#[tokio::test]
async fn archiving_moves_the_store_at_once_and_the_row_leaves_once_its_exit_settles() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    assert!(mailboxes(&store, dana).contains(MailboxRole::Inbox));

    archive_dana(&mut dom, &seen, &store, dana).await;
    settle(&mut dom).await;

    assert!(
        !mailboxes(&store, dana).contains(MailboxRole::Inbox),
        "the store did not have the archive before the row's animation ended"
    );
    let page = dioxus_ssr::render(&dom);
    let going = row_markup(&page, DANA).expect("the row is still drawn while it leaves");
    assert!(
        going.contains("data-presence=\"leaving\"") && going.contains("data-exit=\"row\""),
        "the leaving row is not leaving[data-exit=row]:\n{going}"
    );

    // Nothing the webview says ends it: the roster times the exit on quire's clock. Stop on the
    // render where the row goes, which is the render that starts the heal under it. Waiting the
    // heal out as well would find that row already present.
    let slack = std::time::Duration::from_millis(1000);
    assert!(
        until(&mut dom, exit_settles() + slack, |page| {
            row_markup(page, DANA).is_none()
        })
        .await,
        "the row was still drawn after its exit ended"
    );
    let page = dioxus_ssr::render(&dom);
    let next = row_markup(&page, RELEASE).expect("the next row is there");
    assert!(
        next.contains("data-presence=\"healing\""),
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

    archive_dana(&mut dom, &seen, &store, dana).await;
    assert_ne!(mailboxes(&store, dana), before, "the archive did nothing");
    // The toast is quire's, and its host lays it out a frame after the push: draw until it has,
    // and it names the op and offers the undo.
    let mut page = String::new();
    for _ in 0..8 {
        settle(&mut dom).await;
        page = dioxus_ssr::render(&dom);
        if page.contains("ds-toast-action") {
            break;
        }
    }
    assert!(
        page.contains("class=\"ds-toast-action\""),
        "the archive put up no toast with an Undo:\n{page}"
    );
    chord(
        &mut dom,
        "z",
        crate::ui::fixtures::PRIMARY,
        ElementId(INSIDE_THE_SHELL as usize),
    );

    assert_eq!(
        mailboxes(&store, dana),
        before,
        "undo did not put the thread back where it was"
    );
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
    pointer(&mut dom, "pointermove", row, at(300.0, 160.0));
    pointer(&mut dom, "pointermove", row, at(120.0, 300.0));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("class=\"ds-drag-ghost\""),
        "no ghost follows the pointer"
    );
    pointer(&mut dom, "pointerenter", archive, at(120.0, 300.0));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("data-drop=\"target\" data-place=\"Archive\""),
        "Archive did not light up as the target"
    );
    pointer(&mut dom, "pointerup", archive, at(120.0, 300.0));

    assert!(
        !mailboxes(&store, dana).contains(MailboxRole::Inbox),
        "dropping on Archive did not archive"
    );
}

#[tokio::test]
async fn a_row_dropped_on_a_label_wears_it() {
    let Mounted { mut dom, seen, .. } = mounted_with(|store, dana| {
        let account = store.thread(dana).unwrap().summary.account;
        mail_store::testing::seed_label(
            store,
            LabelId::generate(),
            account.clone(),
            "travel",
            mail_domain::LabelOrigin::User,
        );
    });
    settle(&mut dom).await;
    let row = seen.one("aria-label", &format!("Open {DANA}"));
    let travel = seen.one("data-place", "travel");

    pointer(&mut dom, "pointerdown", row, at(420.0, 120.0));
    pointer(&mut dom, "pointermove", row, at(120.0, 300.0));
    pointer(&mut dom, "pointerenter", travel, at(120.0, 300.0));
    pointer(&mut dom, "pointerup", travel, at(120.0, 300.0));
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    let labelled = row_markup(&page, DANA).expect("Dana's row is drawn");
    assert!(
        labelled.contains(">travel<"),
        "the label dropped on the row is not one of its chips:\n{labelled}"
    );
}

/// The foot menu's rows, after opening it from the button the first paint gave out.
async fn foot_menu(dom: &mut VirtualDom, seen: &Seen) -> (Seen, Vec<String>) {
    let asked = click(dom, seen.one("aria-label", "Sidebar menu"));
    settle(dom).await;
    let opened = crate::ui::fixtures::settle(dom, asked);
    let names = crate::ui::fixtures::menu_names(&dioxus_ssr::render(dom));
    (opened, names)
}

#[tokio::test]
async fn a_today_entry_is_listed_when_opened_gone_when_cleared_and_expires_on_the_clock() {
    let Mounted { mut dom, seen, .. } = mounted();
    // A thread Today does not hold yet: the release thread.
    let (_, names) = foot_menu(&mut dom, &seen).await;
    assert!(
        !names.iter().any(|name| name == RELEASE),
        "already in Today"
    );
    // Close the menu again by its button.
    click(&mut dom, seen.one("aria-label", "Sidebar menu"));
    let row = seen.one("aria-label", &format!("Open {RELEASE}"));
    click(&mut dom, row);
    settle(&mut dom).await;
    let (opened, names) = foot_menu(&mut dom, &seen).await;
    assert!(
        names.iter().any(|name| name == RELEASE),
        "the opened thread is not in Today: {names:?}"
    );
    crate::ui::fixtures::pick_named(&mut dom, &opened, "Clear Today").await;
    settle(&mut dom).await;
    let (_, names) = foot_menu(&mut dom, &seen).await;
    assert!(
        !names.iter().any(|name| name == RELEASE),
        "Clear Today left it: {names:?}"
    );

    // Expiry is the kit's: an entry idle past `IDLE` is not live, whatever was drawn.
    let space = crate::ui::space::SpaceId(0);
    let now = chrono::Utc::now();
    let mut today = crate::ui::today::Today::default();
    let thread = ThreadId::generate();
    today.opened(
        space,
        thread,
        crate::ui::today::at(
            now - chrono::Duration::from_std(crate::ui::today::IDLE).unwrap()
                - chrono::TimeDelta::seconds(5),
        ),
    );
    assert!(today.live(space, crate::ui::today::at(now)).is_empty());
    today.opened(space, thread, crate::ui::today::at(now));
    assert_eq!(today.live(space, crate::ui::today::at(now)).len(), 1);
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
    let places = crate::ui::view::default_places();
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
    let labelled = crate::ui::view::Place {
        name: "spec".to_owned(),
        source: crate::ui::view::Source::Mail(Filter::HasLabel(label)),
        unread: None,
    };
    assert_eq!(
        drop_op(&view_kind(&labelled)),
        Some(Op::Label(label, Membership::In))
    );
}

#[tokio::test]
async fn each_press_on_the_star_flips_the_row_and_only_that() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    let star = seen
        .all("aria-label", "Star this thread")
        .into_iter()
        .chain(seen.all("aria-label", "Unstar this thread"))
        .next()
        .expect("a star");
    let before = store.thread(dana).unwrap().summary.star;
    click(&mut dom, star);
    settle(&mut dom).await;
    let once = store.thread(dana).unwrap().summary.star;
    assert_ne!(once, before, "the first press did not flip the star");
    click(&mut dom, star);
    settle(&mut dom).await;
    assert_eq!(
        store.thread(dana).unwrap().summary.star,
        before,
        "the second press did not flip it back"
    );
}

/// An undo mid-exit takes the exit back (`Roster::stay`): the row never leaves, so the rows
/// below it never close a gap that was never there. Before the stay, the old key finished
/// leaving unseen and the rows under it healed for nothing. The exit it interrupted settles on
/// its own clock, and the row must outlive it, drawn once and no longer leaving.
#[tokio::test]
async fn an_undo_mid_exit_keeps_the_row_once_and_heals_nothing() {
    let Mounted {
        mut dom,
        seen,
        store,
        dana,
        ..
    } = mounted();
    archive_dana(&mut dom, &seen, &store, dana).await;
    settle(&mut dom).await;
    chord(
        &mut dom,
        "z",
        crate::ui::fixtures::PRIMARY,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    let heal = anim_settle(Anim::Heal, MotionLevel::Standard);
    let until = tokio::time::Instant::now() + exit_settles() + heal;
    while tokio::time::Instant::now() < until {
        let left = until - tokio::time::Instant::now();
        let step = left.min(std::time::Duration::from_millis(20));
        if tokio::time::timeout(step, dom.wait_for_work())
            .await
            .is_ok()
        {
            dom.render_immediate(&mut NoOpMutations);
        }
        let page = dioxus_ssr::render(&dom);
        assert!(
            !page.contains("data-presence=\"healing\""),
            "a row healed after an undo brought the row above it back mid-exit:\n{}",
            row_markup(&page, RELEASE).unwrap_or_default()
        );
    }
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    let row = row_markup(&page, DANA).expect("the row an undo brought back is not drawn");
    assert!(
        !row.contains("data-presence=\"leaving\""),
        "the row is still leaving after the undo:\n{row}"
    );
    assert_eq!(
        page.matches(&format!("aria-label=\"Open {DANA}\"")).count(),
        1,
        "the row is drawn twice"
    );
}
