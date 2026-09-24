//! Motion in the running window: a row leaves after the store has already moved, an undo puts
//! the thread back exactly, and a drag onto a place is the same op as the button.

use super::drag::{drop_op, view_kind};
use crate::ui::app::App;
use crate::ui::fixtures::{
    FakePointer, INSIDE_THE_SHELL, Seen, chord, click, dispatching, pointer, rebuild_into, work,
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

/// Draw whatever lands until `span` of quire's clock has passed: a roster's timers run on wall
/// time, and a test waits for them the way the window does.
async fn run_for(dom: &mut VirtualDom, span: std::time::Duration) {
    let until = tokio::time::Instant::now() + span;
    while tokio::time::Instant::now() < until {
        let left = until - tokio::time::Instant::now();
        if tokio::time::timeout(left, dom.wait_for_work())
            .await
            .is_ok()
        {
            dom.render_immediate(&mut NoOpMutations);
        }
    }
    settle(dom).await;
}

/// The longest a row's exit takes to settle at the window's motion level: an unread row's fold.
fn exit_settles() -> std::time::Duration {
    ds::settle(
        ds::Anim::FoldHeavy,
        ds::MotionLevel::Standard,
        ds::StaggerIndex::default(),
    )
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
async fn archiving_moves_the_store_at_once_and_the_row_leaves_once_its_exit_settles() {
    let Mounted {
        mut dom,
        store,
        dana,
        seen,
        ..
    } = mounted();
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
        going.contains("data-presence=\"leaving\"") && going.contains("data-exit=\"fold\""),
        "the leaving row is not leaving[data-exit=fold]:\n{going}"
    );

    // Nothing the webview says ends it: the roster times the exit on quire's clock.
    run_for(&mut dom, exit_settles()).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        row_markup(&page, DANA).is_none(),
        "the row was still drawn after its exit ended"
    );
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

    // The gulp ends on quire's clock, not on the webview's word.
    run_for(
        &mut dom,
        ds::settle(
            ds::Anim::Gulp,
            ds::MotionLevel::Standard,
            ds::StaggerIndex::default(),
        ),
    )
    .await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("item gulp"),
        "the place was still gulping once the gulp had settled"
    );
}

#[tokio::test]
async fn a_label_dropped_on_a_row_lands_until_its_chip_settles() {
    let Mounted { mut dom, seen, .. } = mounted_with(|store, dana| {
        let account = store.thread(dana).unwrap().summary.account;
        store
            .connection()
            .execute(
                "INSERT INTO labels (id, account, name, origin) VALUES (?1, ?2, 'travel', '\"user\"')",
                rusqlite::params![LabelId::generate().to_string(), account.to_string()],
            )
            .unwrap();
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
    let landing = row_markup(&page, DANA).expect("Dana's row is drawn");
    assert!(
        landing.contains("class=\"chip is-landing\" data-chip=\"travel\""),
        "the label does not land on the row:\n{landing}"
    );

    run_for(
        &mut dom,
        ds::settle(
            ds::Anim::ChipLand,
            ds::MotionLevel::Standard,
            ds::StaggerIndex::default(),
        ),
    )
    .await;
    let page = dioxus_ssr::render(&dom);
    let landed = row_markup(&page, DANA).expect("Dana's row is drawn");
    assert!(
        landed.contains("class=\"chip\" data-chip=\"travel\""),
        "the chip was still landing once it had settled:\n{landed}"
    );
}

/// Today's entry for `thread`, as rendered, if it is drawn.
fn today_entry(page: &str, thread: &str) -> Option<String> {
    let at = page.find(&format!("data-hc=\"today:{thread}\""))?;
    let start = page[..at].rfind("<div")?;
    Some(page[start..at].to_owned())
}

#[tokio::test]
async fn a_today_entry_opens_and_closes_on_quires_clock() {
    let Mounted { mut dom, seen, .. } = mounted();
    // A thread Today does not hold yet: the release thread, by the id its row carries.
    let page = dioxus_ssr::render(&dom);
    let release = row_markup(&page, RELEASE).expect("the release row is drawn");
    let opened: String = release
        .split("data-hc=\"thread:")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .map(str::to_owned)
        .expect("the row names its thread");
    assert!(today_entry(&page, &opened).is_none(), "already in Today");
    let row = seen.one("aria-label", &format!("Open {RELEASE}"));
    let mut painted = click(&mut dom, row);
    // Drawn as `settle` would, keeping what was painted: the entry's close button among it.
    for _ in 0..8 {
        if tokio::time::timeout(std::time::Duration::from_millis(40), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        painted = painted.merge(more);
    }
    let page = dioxus_ssr::render(&dom);
    let entry = today_entry(&page, &opened).expect("the opened thread is not in Today");
    assert!(
        entry.contains("class=\"item today-item entering\""),
        "the entry did not open:\n{entry}"
    );
    let span = |anim| ds::settle(anim, ds::MotionLevel::Standard, ds::StaggerIndex::default());
    run_for(&mut dom, span(ds::Anim::TabIn)).await;
    let page = dioxus_ssr::render(&dom);
    let entry = today_entry(&page, &opened).expect("the entry is drawn");
    assert!(
        entry.contains("class=\"item today-item\""),
        "the entry was still opening once its entrance had settled:\n{entry}"
    );

    let close = seen
        .merge(painted)
        .one("aria-label", &format!("Close {opened}"));
    click(&mut dom, close);
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    let entry = today_entry(&page, &opened).expect("a closed entry is drawn while it goes");
    assert!(
        entry.contains("class=\"item today-item leaving\""),
        "the entry did not close:\n{entry}"
    );
    run_for(&mut dom, span(ds::Anim::TabOut)).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        today_entry(&page, &opened).is_none(),
        "the entry was still drawn once its exit had settled"
    );
}

#[tokio::test]
async fn a_count_that_changes_bumps() {
    let Mounted { mut dom, seen, .. } = mounted();
    settle(&mut dom).await;
    // The Inbox's count, with the pulse it is playing, if any.
    let inbox = |page: &str| -> String {
        let at = page.find("data-place=\"Inbox\"").expect("the Inbox place");
        let end = page[at..].find("</button>").map_or(page.len(), |n| at + n);
        let tail = &page[at..end];
        tail.find("<span class=\"count")
            .map(|n| tail[n..].split('>').next().unwrap_or("").to_owned())
            .expect("the Inbox shows a count")
    };
    let before = inbox(&dioxus_ssr::render(&dom));
    click(&mut dom, dana_archive(&seen));
    settle(&mut dom).await;
    let after = inbox(&dioxus_ssr::render(&dom));
    assert!(
        after.contains("class=\"count a-bump\"") && after != before,
        "the Inbox's count changed and did not bump: {before} then {after}"
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

#[tokio::test]
async fn each_press_on_the_star_replays_its_pop_and_sparks() {
    let Mounted { mut dom, seen, .. } = mounted();
    let page = dioxus_ssr::render(&dom);
    let row = row_markup(&page, DANA).expect("Dana's row is drawn");
    assert!(
        row.contains("class=\"star-ic\"") && !row.contains("a-spark"),
        "the star plays before it is pressed:\n{row}"
    );
    let star = seen
        .all("aria-label", "Star")
        .into_iter()
        .chain(seen.all("aria-label", "Unstar"))
        .next()
        .expect("a star");
    let mut aliases = Vec::new();
    for _ in 0..2 {
        click(&mut dom, star);
        settle(&mut dom).await;
        let page = dioxus_ssr::render(&dom);
        let pressed = page
            .find("class=\"star-ic a-star-pop\" data-pulse=\"")
            .map(|at| &page[at + "class=\"star-ic a-star-pop\" data-pulse=\"".len()..][..1])
            .expect("the pressed star does not pop")
            .to_owned();
        assert!(
            page.contains(&format!("class=\"a-spark\" data-pulse=\"{pressed}\"")),
            "the sparks do not fly with the pop"
        );
        aliases.push(pressed);
    }
    // quire restarts a keyframe by swapping to its other name; the same name would not replay.
    assert_eq!(
        aliases,
        ["a", "b"],
        "the second press did not restart the pop"
    );
}

#[tokio::test]
async fn an_undo_mid_exit_brings_the_row_back_for_good() {
    let Mounted { mut dom, seen, .. } = mounted();
    click(&mut dom, dana_archive(&seen));
    settle(&mut dom).await;
    chord(
        &mut dom,
        "z",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    // The exit it interrupted settles on its own clock; the row must outlive it.
    run_for(&mut dom, exit_settles()).await;
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

#[tokio::test]
async fn the_list_rises_when_shown_and_rests_once_its_rows_have_arrived() {
    let Mounted { mut dom, .. } = mounted();
    settle(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("class=\"list\" data-presence=\"entering\""),
        "the list is not being shown on its first frame"
    );
    let row = row_markup(&page, DANA).expect("Dana's row is drawn");
    assert!(
        row.contains("data-presence=\"entering\""),
        "the first row is not arriving:\n{row}"
    );

    // The roster rests its first rows once the last of them has risen, on quire's clock; the
    // list rests with them.
    let last = ds::settle(
        ds::Anim::RowIn,
        ds::MotionLevel::Standard,
        ds::StaggerIndex::new(12),
    );
    run_for(&mut dom, last).await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("class=\"list\" data-presence=\"present\""),
        "the list is still being shown after its rows arrived"
    );
    let row = row_markup(&page, DANA).expect("Dana's row is drawn");
    assert!(
        row.contains("data-presence=\"present\""),
        "the first row did not come to rest:\n{row}"
    );
}
