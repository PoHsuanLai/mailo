//! Motion in the running window: a row leaves after the store has already moved, an undo puts
//! the thread back exactly, and a drag onto a place is the same op as the button.

use super::drag::{drop_op, view_kind};
use crate::ui::app::App;
use crate::ui::fixtures::{
    FakePointer, INSIDE_THE_SHELL, Seen, chord, click, dispatching, open_row_menu, pick_until,
    pointer, rebuild_into, work,
};
use crate::ui::fixtures::{settle_quiet, settle_until, step_clock, step_clock_until};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use ds::base::time::clock::VirtualClock;
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
        .with_root_context(crate::ui::fixtures::client(&built.store))
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
        settle_quiet(&mut dom).await;
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
    settle_quiet(&mut dom).await;
    let row = seen.one("aria-label", &format!("Open {DANA}"));
    let travel = seen.one("data-place", "travel");

    pointer(&mut dom, "pointerdown", row, at(420.0, 120.0));
    pointer(&mut dom, "pointermove", row, at(120.0, 300.0));
    pointer(&mut dom, "pointerenter", travel, at(120.0, 300.0));
    pointer(&mut dom, "pointerup", travel, at(120.0, 300.0));
    settle_quiet(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    let labelled = row_markup(&page, DANA).expect("Dana's row is drawn");
    assert!(
        labelled.contains(">travel<"),
        "the label dropped on the row is not one of its chips:\n{labelled}"
    );
}

/// Draw until no background work is left, keeping what the renders set: a Today row drawn after
/// the first paint is found by its id.
async fn drawn(dom: &mut VirtualDom, mut seen: Seen) -> Seen {
    for _ in 0..8 {
        if tokio::time::timeout(std::time::Duration::from_millis(40), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    seen
}

/// The page's words with its tags taken out, from `from` on.
fn words_after(page: &str, from: &str) -> String {
    let mut words = String::new();
    let mut inside = false;
    for ch in page.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            ch if !inside => words.push(ch),
            _ => {}
        }
    }
    words
        .split_once(from)
        .map(|(_, rest)| rest.to_owned())
        .unwrap_or_default()
}

#[tokio::test]
async fn an_opened_conversation_is_in_command_k_recent_and_never_leaves_on_the_clock() {
    let Mounted { mut dom, seen, .. } = mounted();
    let row = seen.one("aria-label", &format!("Open {RELEASE}"));
    let opened = click(&mut dom, row);
    drawn(&mut dom, opened).await;
    // ⌘K with nothing typed: Recent is where the person was.
    chord(
        &mut dom,
        "k",
        crate::ui::fixtures::PRIMARY,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    settle_quiet(&mut dom).await;
    let recent = words_after(&dioxus_ssr::render(&dom), "Recent");
    let recent = recent.split("Commands").next().unwrap_or_default();
    assert!(
        recent.contains(RELEASE),
        "the opened conversation is not in Recent: {recent}"
    );

    // Nothing drops off by time: an entry idle past the kit's `IDLE` is still listed.
    let space = crate::ui::space::SpaceId(0);
    let now = chrono::Utc::now();
    let mut today = crate::ui::today::Today::default();
    let thread = ThreadId::generate();
    today.opened(
        space,
        thread,
        crate::ui::today::at(
            now - chrono::Duration::from_std(ds::components::app::spaces::IDLE).unwrap()
                - chrono::TimeDelta::seconds(5),
        ),
    );
    assert_eq!(today.all(space).len(), 1);
    today.opened(space, thread, crate::ui::today::at(now));
    assert_eq!(
        today.all(space).len(),
        1,
        "opening it again keeps one entry"
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
    let places = crate::ui::view::default_places();
    let label = LabelId::generate();
    let cases: Vec<(&str, Option<Op>)> = vec![
        ("Inbox", Some(Op::Restore)),
        ("Archive", Some(Op::Archive)),
        ("Trash", Some(Op::Trash)),
        ("Spam", Some(Op::Spam)),
        // Saved searches and places nothing is filed into refuse.
        ("Starred", None),
        ("Pinned", None),
        ("History", None),
        ("Snoozed", None),
        ("Waiting", None),
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

/// A star is drawn only on a starred conversation, as its mark (starring is in the row's menu
/// and on its key), and pressing it unstars that conversation and only that one.
#[tokio::test]
async fn the_star_marks_only_starred_rows_and_a_press_unstars_that_one() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
    assert!(
        seen.all("aria-label", "Star this thread").is_empty(),
        "an unstarred row draws a star"
    );
    let stars = seen.all("aria-label", "Unstar this thread");
    assert_eq!(stars.len(), 1, "the fixture has one starred conversation");
    let sam = crate::ui::fixtures::thread_like(&store, "Notes from the sync review");
    assert_eq!(store.thread(sam).unwrap().summary.star, Star::Starred);
    click(&mut dom, stars[0]);
    settle_quiet(&mut dom).await;
    assert_eq!(
        store.thread(sam).unwrap().summary.star,
        Star::Unstarred,
        "the press did not unstar it"
    );
    assert_eq!(
        store.thread(dana).unwrap().summary.star,
        Star::Unstarred,
        "another row changed"
    );
}

/// An undo mid-exit takes the exit back (`Roster::stay`): the row never leaves, so the rows
/// below it never close a gap that was never there. Before the stay, the old key finished
/// leaving unseen and the rows under it healed for nothing. The exit it interrupted settles on
/// its own clock, and the row must outlive it, drawn once and no longer leaving.
///
/// Quire's clock is the test's: the undo lands while the exit is under way however long the
/// machine takes to get there, and only then do the exit's and the heal's timers run.
#[tokio::test]
async fn an_undo_mid_exit_keeps_the_row_once_and_heals_nothing() {
    let clock = VirtualClock::new();
    let _installed = clock.install();
    let Mounted {
        mut dom,
        seen,
        store,
        dana,
        ..
    } = mounted();
    let opened = open_row_menu(&mut dom, &seen, DANA);
    pick_until(&mut dom, &opened, "Archive", || true).await;
    let archived = || !mailboxes(&store, dana).contains(MailboxRole::Inbox);
    step_clock_until(&mut dom, &clock, std::time::Duration::from_secs(2), |_| {
        archived()
    })
    .await;
    let leaving = |dom: &VirtualDom| {
        row_markup(&dioxus_ssr::render(dom), DANA)
            .is_some_and(|row| row.contains("data-presence=\"leaving\""))
    };
    settle_until(
        &mut dom,
        &mut Seen::default(),
        std::time::Duration::from_millis(80),
        std::time::Duration::from_secs(10),
        leaving,
    )
    .await;
    chord(
        &mut dom,
        "z",
        crate::ui::fixtures::PRIMARY,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    settle_until(
        &mut dom,
        &mut Seen::default(),
        std::time::Duration::from_millis(80),
        std::time::Duration::from_secs(10),
        |dom| !archived() && !leaving(dom),
    )
    .await;
    let heal = anim_settle(Anim::Heal, MotionLevel::Standard);
    step_clock(&mut dom, &clock, exit_settles() + heal, |page| {
        assert!(
            !page.contains("data-presence=\"healing\""),
            "a row healed after an undo brought the row above it back mid-exit:\n{}",
            row_markup(page, RELEASE).unwrap_or_default()
        );
    })
    .await;
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
