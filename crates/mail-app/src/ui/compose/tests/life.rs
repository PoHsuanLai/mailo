//! The page against a real store: the guards, autosave, Esc, and Send then Undo.

use ds::prelude::*;
use mail_core::Store;

use super::super::desk::reopen;
use super::super::page::{Guard, Phase};
use super::*;
use crate::ui::fixtures::{acct_account, click, press, seeded};
use ds_harness::harness::SETTLE_BOUND;
use mail_core::SqliteStore;
use mail_core::rich_text::{to_flowed, to_html};

fn far() -> DateTime<Utc> {
    Utc::now() + chrono::TimeDelta::days(365)
}

fn fresh_draft(store: &SqliteStore) -> Draft {
    mail_core::compose::draft_new(store, acct_account(), &[], "", "", Utc::now())
        .unwrap_or_else(|why| panic!("a new draft: {why}"))
}

fn queued(store: &SqliteStore) -> usize {
    store
        .outbox_due(acct_account(), far())
        .unwrap_or_else(|why| panic!("the outbox: {why}"))
        .len()
}

/// An autosave already under way when the draft is discarded (from Drafts, a template, an
/// action) does not write it back.
#[test]
fn an_autosave_taken_before_a_discard_does_not_bring_the_draft_back() {
    use super::super::life;
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let page = life::load(
        &crate::ui::fixtures::client(&store),
        draft.id,
        &mut Vec::new(),
        &crate::settings::ComposeSettings::default(),
    )
    .expect("the draft's page");
    let taken = life::stamp();
    life::discard(&crate::ui::fixtures::client(&store), draft.id)
        .unwrap_or_else(|why| panic!("discard: {why}"));
    let saved = life::save_stamped(
        &crate::ui::fixtures::client(&store),
        &page,
        taken,
        Utc::now(),
    );
    assert!(matches!(saved, Ok(None)), "{saved:?}");
    assert!(store.draft(draft.id).is_err(), "the draft came back");
}

/// Autosave asks for a copy on the server at most once every [`life::KEEP_EVERY`]; closing or
/// parking the page asks for one whenever.
#[test]
fn a_copy_for_the_server_is_due_at_most_every_thirty_seconds_unless_the_page_is_going() {
    use super::super::life::{KEEP_EVERY, Keeping, Kept};
    let mut kept = Kept::default();
    let (draft, other) = (DraftId::generate(), DraftId::generate());
    let start = Utc::now();
    let soon = start + chrono::TimeDelta::seconds(5);
    assert!(kept.due(draft, start, Keeping::WhenDue), "the first save");
    assert!(!kept.due(draft, soon, Keeping::WhenDue), "five seconds on");
    assert!(
        kept.due(other, soon, Keeping::WhenDue),
        "each draft keeps its own time"
    );
    assert!(kept.due(draft, soon, Keeping::Now), "the page is closing");
    let after = soon + KEEP_EVERY;
    assert!(
        !kept.due(
            draft,
            after - chrono::TimeDelta::seconds(1),
            Keeping::WhenDue
        ),
        "counted from the copy the closing page kept"
    );
    assert!(
        kept.due(draft, after, Keeping::WhenDue),
        "thirty seconds on"
    );
}

/// An autosave on an account whose server names a Drafts folder queues the draft's copy for it,
/// and one made a moment later does not queue another.
#[test]
fn an_autosave_queues_the_draft_s_copy_for_the_server_s_drafts_folder() {
    use super::super::life::{self, Keeping};
    let (store, _dir) = seeded();
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example.test".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    store.set_account_plan(acct_account(), &plan).unwrap();
    let caps = AccountCaps {
        archive: mail_domain::ArchiveMeans::LocalOnly,
        folders: FolderRoles(vec![("Drafts".to_owned(), MailboxRole::Drafts)]),
        ..crate::ui::fixtures::gmail_caps()
    };
    store.put_caps(acct_account(), &caps, Utc::now()).unwrap();
    let draft = fresh_draft(&store);
    let mut page = life::load(
        &crate::ui::fixtures::client(&store),
        draft.id,
        &mut Vec::new(),
        &crate::settings::ComposeSettings::default(),
    )
    .expect("the draft's page");
    page.subject = "half a thought".to_owned();
    let now = Utc::now();
    life::save(&crate::ui::fixtures::client(&store), &mut page, now).unwrap();

    let keeps = |store: &SqliteStore| {
        store
            .outbox_due(acct_account(), far())
            .unwrap()
            .into_iter()
            .filter(
                |entry| matches!(&entry.op, ProtoOp::KeepDraft { draft: d, .. } if *d == draft.id),
            )
            .count()
    };
    assert_eq!(
        life::keep_on_server(
            &crate::ui::fixtures::client(&store),
            draft.id,
            now,
            Keeping::WhenDue
        ),
        Ok(true)
    );
    assert_eq!(keeps(&store), 1);
    let soon = now + chrono::TimeDelta::seconds(5);
    assert_eq!(
        life::keep_on_server(
            &crate::ui::fixtures::client(&store),
            draft.id,
            soon,
            Keeping::WhenDue
        ),
        Ok(false),
        "a second copy five seconds later"
    );
    // Closing the page keeps one now, in place of the one still waiting.
    assert_eq!(
        life::keep_on_server(
            &crate::ui::fixtures::client(&store),
            draft.id,
            soon,
            Keeping::Now
        ),
        Ok(true)
    );
    assert_eq!(keeps(&store), 1);
}

#[tokio::test]
async fn send_with_nobody_to_send_to_queues_nothing_and_marks_the_to_field_invalid() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let before = queued(&store);
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let mut page = window.page();
    window
        .dom
        .in_runtime(|| type_text(&mut page.write(), "hello"));
    let untouched = window.render();
    assert!(
        !untouched.contains(r#"data-validity="invalid""#),
        "the To field was refused before Send was pressed:\n{untouched}"
    );

    let send = seen.one("aria-label", crate::ui::copy::controls::SEND);
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
        markup.contains(r#"data-validity="invalid""#)
            && markup.contains("Add at least one person to send to."),
        "the To field was not marked and did not say why:\n{markup}"
    );

    // A second press is a new refusal: the guard counts it, so quire's field treats it as one.
    click(&mut window.dom, send);
    let markup = window.render();
    assert_eq!(
        window.dom.in_runtime(|| page.peek().guard),
        Guard::NoRecipient(2),
        "{markup}"
    );
    assert_eq!(queued(&store), before);
}

#[tokio::test]
async fn an_attachment_mentioned_and_missing_shows_one_bar_until_send_anyway() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let before = queued(&store);
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        type_text(&mut write, "I attached the agenda.");
    });
    let markup = window.render();
    assert!(
        !markup.contains("ds-inline-banner"),
        "the bar showed before Send"
    );

    let after_send = click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
    let markup = window.render();
    assert_eq!(
        markup.matches(r#"class="ds-inline-banner""#).count(),
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
async fn attach_puts_the_files_the_dialog_chose_on_the_draft() {
    use crate::ui::pick::{Ask, Dialogs};
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let files = tempfile::tempdir().unwrap_or_else(|why| panic!("a temp dir: {why}"));
    let agenda = files.path().join("agenda.txt");
    std::fs::write(&agenda, "Friday, ten o'clock").unwrap_or_else(|why| panic!("{why}"));
    // Over the budget without writing it: a sparse file is only its length.
    let huge = files.path().join("huge.bin");
    std::fs::File::create(&huge)
        .and_then(|file| file.set_len(mail_core::compose::ATTACHMENT_BUDGET + 1))
        .unwrap_or_else(|why| panic!("{why}"));
    let (dialogs, asked) = Dialogs::answering(vec![agenda, huge]);

    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    window.dom.provide_root_context(dialogs);
    click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::ATTACH),
    );
    for _ in 0..40 {
        let quiet = std::time::Duration::from_millis(150);
        if tokio::time::timeout(quiet, window.dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        window.dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }

    assert_eq!(
        *asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        [(Ask::Attachments, None)]
    );
    let page = window.page();
    let (attached, notice) = window
        .dom
        .in_runtime(|| (page.peek().attached.clone(), page.peek().notice.clone()));
    let names: Vec<&str> = attached.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["agenda.txt"], "{attached:?}");
    assert_eq!(notice.as_deref(), Some("huge.bin is too large to send."));
    let stored = store
        .draft(draft.id)
        .unwrap_or_else(|why| panic!("the draft: {why}"));
    assert_eq!(stored.attachments.len(), 1);
}

#[tokio::test]
async fn autosave_writes_the_documents_two_bodies_once_typing_stops() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let (mut window, _seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        type_text(&mut write, "Notes for Friday");
        write.session.caret = mail_core::rich_text::Caret::at(0, 0);
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
    // Autosave is silent: the page says nothing about it (the bar holds Focus alone).
    let markup = window.render();
    assert!(!markup.contains("saved just now"), "{markup}");
}

/// A composer closed with nothing put in it leaves nothing behind: the draft is thrown away and
/// Today lists nothing to come back to.
#[tokio::test]
async fn esc_on_a_draft_with_nothing_in_it_throws_it_away() {
    let (store, _dir) = seeded();
    let root = tempfile::tempdir().unwrap_or_else(|why| panic!("{why}"));
    let dirs = WindowDirs {
        config: root.path().join("config"),
        state: root.path().join("state"),
    };
    let draft = fresh_draft(&store);
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        Some(dirs.clone()),
    );
    let root_id = seen.one("class", "cpage");
    press(
        &mut window.dom,
        "Escape",
        u32::try_from(root_id.0).unwrap_or(0),
    );
    window.render();
    let shell = window.shell;
    assert!(
        window.dom.in_runtime(|| shell.peek().composing.is_none()),
        "Esc left the page open"
    );
    assert!(store.draft(draft.id).is_err(), "the empty draft was kept");
    assert!(
        crate::ui::today::load(&dirs.state)
            .parked_in(crate::ui::space::SpaceId(0))
            .is_empty(),
        "Today lists the empty draft"
    );
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
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        Some(dirs.clone()),
    );
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.subject = "Offsite".to_owned();
        type_text(&mut write, "Underlined ");
        write.selection = Some(mail_core::rich_text::Range {
            start: Pos::new(0, 0),
            end: Pos::new(0, 10),
        });
    });
    window.dom.in_runtime(|| {
        super::super::body::format(page, "formatUnderline");
    });
    let before = window.dom.in_runtime(|| to_html(&page.peek().session.doc));
    assert!(before.contains("<u>"), "{before}");
    assert!(crate::ui::today::load(&dirs.state).parked.is_empty());

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
    let today = crate::ui::today::load(&dirs.state);
    let parked: Vec<(DraftId, String)> = today
        .parked_in(crate::ui::space::SpaceId(0))
        .iter()
        .map(|p| (p.item, p.title.clone()))
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
        crate::ui::today::load(&dirs.state)
            .parked_in(crate::ui::space::SpaceId(0))
            .is_empty(),
        "reopening left the entry in Today"
    );
}

/// Draw what lands until `done` holds of the markup, for at most `bound` of quire's clock. The
/// markup `done` last saw, and when it first held (or when the bound ran out).
async fn run_until(
    window: &mut Window,
    bound: std::time::Duration,
    done: impl Fn(&str) -> bool,
) -> (String, tokio::time::Instant) {
    let until = tokio::time::Instant::now() + bound;
    loop {
        let markup = window.render();
        let now = tokio::time::Instant::now();
        if done(&markup) || now >= until {
            return (markup, now);
        }
        let _ = tokio::time::timeout(until - now, window.dom.wait_for_work()).await;
    }
}

#[tokio::test]
async fn a_sent_page_folds_away_on_quires_clock() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        write.subject = "Friday".to_owned();
        type_text(&mut write, "See you then.");
    });
    window.render();

    let sent = tokio::time::Instant::now();
    click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
    let markup = window.render();
    assert!(
        markup.contains(r#"class="cpage sending""#),
        "the page did not fold:\n{markup}"
    );

    // Nothing the window says takes it away: the fold's timer does, once the fade has
    // settled. Waited for, not timed: the timer's thread can wake late.
    let fold = settle(Anim::Fade, MotionLevel::Standard);
    let (markup, gone) = run_until(&mut window, fold + SETTLE_BOUND, |markup| {
        !markup.contains("cpage")
    })
    .await;
    let took = gone - sent;
    assert!(
        !markup.contains("cpage"),
        "the page was still drawn {took:?} after Send, past its fold of {fold:?}:\n{markup}"
    );
    assert!(
        took >= fold,
        "the page went after {took:?}, before its fold of {fold:?} had settled"
    );
    assert!(
        markup.contains("Sending in"),
        "the pill went with the page:\n{markup}"
    );
}

/// Send waits as long as `compose.undo_send` says, read from `settings.toml` when it is pressed.
#[tokio::test]
async fn send_waits_as_long_as_the_undo_send_setting_says() {
    let (store, dir) = seeded();
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    crate::settings::change(&crate::settings::root_for(&dirs.config), |settings| {
        settings.compose.undo_send = crate::settings::UndoSend::Seconds30;
    })
    .unwrap_or_else(|why| panic!("{why}"));
    let draft = fresh_draft(&store);
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store),
        draft.clone(),
        Some(dirs),
    );
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        type_text(&mut write, "See you then.");
    });
    window.render();
    let pressed = Utc::now();
    click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
    let markup = window.render();
    assert!(
        markup.contains("Sending in 30 s"),
        "the countdown is not the setting's:\n{markup}"
    );
    let due = |at: DateTime<Utc>| {
        store
            .outbox_due(acct_account(), at)
            .map(|due| due.len())
            .unwrap_or_else(|why| panic!("the outbox: {why}"))
    };
    // Still held where the five seconds the composer once had are long gone, and free to leave
    // once the thirty are.
    assert_eq!(due(pressed + chrono::TimeDelta::seconds(25)), 0);
    assert_eq!(due(Utc::now() + chrono::TimeDelta::seconds(31)), 1);
}

#[tokio::test]
async fn send_then_undo_withdraws_the_submission_and_restores_the_draft() {
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let before_queue = queued(&store);
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        write.subject = "Friday".to_owned();
        type_text(&mut write, "See you then.");
    });
    window.render();
    let before = window.dom.in_runtime(|| page.peek().clone());

    let painted = click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
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
            .outbox_due(acct_account(), Utc::now())
            .map(|due| due.len())
            .ok(),
        Some(0)
    );

    // quire's pill names its button by its word alone, a static class and its text.
    click(
        &mut window.dom,
        painted.fixed("class", "ds-send-pill-undo")[0],
    );
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

/// A draft with a picture in its body reopens with the picture drawn in place, and is written
/// back naming it by its part, which stays on the draft.
#[tokio::test]
async fn a_picture_in_the_body_reopens_drawn_and_is_saved_by_its_name() {
    use super::super::life;
    use mail_core::rich_text::{Node, Object};
    let (store, _dir) = seeded();
    let draft = fresh_draft(&store);
    let png = b"\x89PNG\r\n\x1a\nnot really a picture";
    let (mut stored, cid) =
        mail_core::compose::attach_inline(&store, draft.id, "photo.png", png, Utc::now())
            .unwrap_or_else(|why| panic!("the picture: {why}"));
    stored.html = Some(format!("<p>Look</p><img alt=\"\" src=\"cid:{cid}\">"));
    mail_core::compose::save(&store, &stored).unwrap_or_else(|why| panic!("save: {why}"));

    let page = life::load(
        &crate::ui::fixtures::client(&store),
        draft.id,
        &mut Vec::new(),
        &crate::settings::ComposeSettings::default(),
    )
    .expect("the draft's page");
    let shown = page.session.doc.nodes.iter().any(|node| {
        matches!(node, Node::Object(Object::Image { src, .. }) if src.cid() == Some(cid.as_str()))
    });
    assert!(shown, "{:?}", page.session.doc.nodes);
    assert!(
        page.images
            .get(&cid)
            .is_some_and(|uri| uri.starts_with("data:image/png;base64,")),
        "{:?}",
        page.images
    );
    assert!(page.attached.is_empty(), "the picture is listed as a file");
    let written = page.apply_to(&stored, Utc::now());
    assert!(
        written
            .html
            .as_deref()
            .is_some_and(|html| html.contains(&format!("src=\"cid:{cid}\""))),
        "{:?}",
        written.html
    );
    assert_eq!(written.attachments, stored.attachments);

    let (mut window, _seen) = Window::open(crate::ui::fixtures::client(&store), stored, None);
    let markup = window.render();
    assert!(
        markup.contains("<img src=\"data:image/png;base64,"),
        "the picture is not drawn:\n{markup}"
    );
}
