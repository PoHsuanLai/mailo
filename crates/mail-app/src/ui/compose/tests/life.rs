//! The page against a real store: the guards, autosave, Esc, and Send then Undo.

use mail_store::Store;

use super::super::desk::reopen;
use super::super::page::{Guard, Phase};
use super::*;
use crate::editor::{to_flowed, to_html};
use crate::ui::fixtures::{ACCOUNT, click, press, seeded};

fn far() -> DateTime<Utc> {
    Utc::now() + chrono::TimeDelta::days(365)
}

fn fresh_draft(store: &SqliteStore) -> Draft {
    crate::compose::draft_new(store, ACCOUNT, &[], "", "", Utc::now())
        .unwrap_or_else(|why| panic!("a new draft: {why}"))
}

fn queued(store: &SqliteStore) -> usize {
    store
        .outbox_due(ACCOUNT, far())
        .unwrap_or_else(|why| panic!("the outbox: {why}"))
        .len()
}

#[tokio::test]
async fn send_with_nobody_to_send_to_queues_nothing_and_shakes_the_to_row() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let before = queued(&store);
    let (mut window, seen) = Window::open(store.clone(), draft.clone(), None);
    let mut page = window.page();
    window
        .dom
        .in_runtime(|| type_text(&mut page.write(), "hello"));
    window.render();

    let send = seen.one("aria-label", "Send");
    click(&mut window.dom, send);
    let markup = window.render();
    assert_eq!(
        queued(&store),
        before,
        "a send with no recipient reached the outbox"
    );
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Editing),
        "the draft moved"
    );
    assert!(
        markup.contains(r#"class="prop-row shake""#),
        "the To row did not shake:\n{markup}"
    );

    // A second press shakes it again: the class changes so the animation restarts.
    click(&mut window.dom, send);
    let markup = window.render();
    assert!(
        markup.contains(r#"class="prop-row shake again""#),
        "{markup}"
    );
    assert_eq!(queued(&store), before);
}

#[tokio::test]
async fn an_attachment_mentioned_and_missing_shows_one_bar_until_send_anyway() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let before = queued(&store);
    let (mut window, seen) = Window::open(store.clone(), draft.clone(), None);
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        type_text(&mut write, "I attached the agenda.");
    });
    let markup = window.render();
    assert!(!markup.contains("c-warn"), "the bar showed before Send");

    let after_send = click(&mut window.dom, seen.one("aria-label", "Send"));
    let markup = window.render();
    assert_eq!(
        markup.matches(r#"class="c-warn""#).count(),
        1,
        "one warning bar:\n{markup}"
    );
    assert!(
        markup.contains("Attach a file") && markup.contains("Send anyway"),
        "{markup}"
    );
    assert_eq!(queued(&store), before, "the guard let it through");
    assert_eq!(window.dom.in_runtime(|| page.peek().guard), Guard::Warn);

    click(&mut window.dom, after_send.one("aria-label", "Send anyway"));
    assert_eq!(queued(&store), before + 1, "Send anyway did not send");
}

#[tokio::test]
async fn autosave_writes_the_documents_two_bodies_once_typing_stops() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let (mut window, _seen) = Window::open(store.clone(), draft.clone(), None);
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        type_text(&mut write, "Notes for Friday");
        write.session.caret = crate::editor::Caret::at(0, 0);
        type_text(&mut write, "# ");
    });
    window.render();
    let doc = window.dom.in_runtime(|| page.peek().session.doc.clone());
    assert_ne!(
        store.draft(draft.id).map(|d| d.text).ok(),
        Some(to_flowed(&doc)),
        "saved already"
    );

    let mut stored = None;
    for _ in 0..40 {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            window.dom.wait_for_work(),
        )
        .await;
        window.dom.render_immediate(&mut NoOpMutations);
        let now = store.draft(draft.id).unwrap_or_else(|why| panic!("{why}"));
        if now.text == to_flowed(&doc) {
            stored = Some(now);
            break;
        }
    }
    let stored = stored.unwrap_or_else(|| panic!("autosave never wrote the draft"));
    assert_eq!(stored.html.as_deref(), Some(to_html(&doc).as_str()));
    assert!(
        stored
            .html
            .as_deref()
            .is_some_and(|html| html.starts_with("<h1>")),
        "{:?}",
        stored.html
    );
    let markup = window.render();
    assert!(markup.contains("saved just now"), "{markup}");
}

#[tokio::test]
async fn esc_parks_the_draft_in_today_and_the_entry_brings_it_back_exactly() {
    let (store, _dir) = seeded();
    let root = tempfile::tempdir().unwrap_or_else(|why| panic!("{why}"));
    let dirs = WindowDirs {
        config: root.path().join("config"),
        state: root.path().join("state"),
    };
    let draft = fresh_draft(&store);
    let (mut window, seen) = Window::open(store.clone(), draft.clone(), Some(dirs.clone()));
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.subject = "Offsite".to_owned();
        type_text(&mut write, "Underlined ");
        write.selection = Some(crate::editor::Range {
            start: Pos::new(0, 0),
            end: Pos::new(0, 10),
        });
    });
    window.dom.in_runtime(|| {
        super::super::body::format(page, "formatUnderline");
    });
    let before = window.dom.in_runtime(|| to_html(&page.peek().session.doc));
    assert!(before.contains("<u>"), "{before}");
    assert!(crate::today::load(&dirs.state).drafts.is_empty());

    let root_id = seen.one("class", "cpage");
    press(
        &mut window.dom,
        "Escape",
        u32::try_from(root_id.0).unwrap_or(0),
    );
    window.render();
    let (shell, desk) = (window.shell, window.desk);
    assert!(
        window.dom.in_runtime(|| shell.peek().composing.is_none()),
        "Esc left the page open"
    );
    let today = crate::today::load(&dirs.state);
    let parked: Vec<(DraftId, String)> = today
        .parked(0)
        .iter()
        .map(|p| (p.draft, p.title.clone()))
        .collect();
    assert_eq!(
        parked,
        [(draft.id, "Offsite".to_owned())],
        "the Today file has no draft entry"
    );

    window
        .dom
        .in_scope(dioxus_core::ScopeId::APP, || reopen(desk, shell, draft.id));
    window.render();
    let page = window.page();
    let after = window.dom.in_runtime(|| to_html(&page.peek().session.doc));
    assert_eq!(
        after, before,
        "the reopened draft is not the one that was parked"
    );
    assert!(
        crate::today::load(&dirs.state).parked(0).is_empty(),
        "reopening left the entry in Today"
    );
}

#[tokio::test]
async fn send_then_undo_withdraws_the_submission_and_restores_the_draft() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let before_queue = queued(&store);
    let (mut window, seen) = Window::open(store.clone(), draft.clone(), None);
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        write.subject = "Friday".to_owned();
        type_text(&mut write, "See you then.");
    });
    window.render();
    let before = window.dom.in_runtime(|| page.peek().clone());

    let painted = click(&mut window.dom, seen.one("aria-label", "Send"));
    let markup = window.render();
    assert_eq!(queued(&store), before_queue + 1, "Send queued nothing");
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Queued)
    );
    assert!(
        markup.contains("Sending in 5 s"),
        "no countdown pill:\n{markup}"
    );
    assert!(
        markup.contains(r#"class="cpage sending""#),
        "the page did not fold:\n{markup}"
    );
    // Nothing leaves before the grace period is over.
    assert_eq!(
        store
            .outbox_due(ACCOUNT, Utc::now())
            .map(|due| due.len())
            .ok(),
        Some(0)
    );

    click(&mut window.dom, painted.one("aria-label", "Undo"));
    let markup = window.render();
    assert_eq!(
        queued(&store),
        before_queue,
        "Undo left the submission in the outbox"
    );
    let restored = store.draft(draft.id).unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(restored.state, SendState::Editing);
    assert_eq!(restored.to.len(), 1);
    let page = window.page();
    let (html, to, phase) = window.dom.in_runtime(|| {
        let read = page.peek();
        (to_html(&read.session.doc), read.to.clone(), read.phase)
    });
    assert_eq!(html, to_html(&before.session.doc));
    assert_eq!(to, before.to);
    assert_eq!(phase, Phase::Writing);
    assert!(
        !markup.contains("Sending in"),
        "the pill stayed after Undo:\n{markup}"
    );
    assert!(
        markup.contains(r#"class="cpage""#),
        "the page is not back:\n{markup}"
    );
}
