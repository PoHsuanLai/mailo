//! The search in the whole window: ⌘K or the toolbar's magnifier brings up the panel with the
//! keyboard in its field, the toolbar holds no field while idle, and a search the panel leaves
//! behind is shown in the toolbar until it is cleared.

use super::bar_tests::{has, until, words};
use super::sections::{COMMANDS, MAIL, PLACES, RECENT, TOP};
use super::*;
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, chord, click, dispatching, drain_seen, rebuild_into, type_into, work,
};
use crate::ui::host::{Ask, Recorder};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus_core::{ElementId, VirtualDom};

/// The window over the reference fixture, with a recorder for what it asks of its host.
fn window(recorder: &Recorder) -> (VirtualDom, crate::ui::fixtures::Work, Seen) {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(recorder.host());
    let seen = rebuild_into(&mut dom);
    (dom, built, seen)
}

#[tokio::test]
async fn command_k_brings_up_the_panel_with_the_keyboard_in_its_field() {
    let recorder = Recorder::default();
    let (mut dom, _built, _seen) = window(&recorder);
    chord(
        &mut dom,
        "k",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    assert!(
        recorder.asked().contains(&Ask::FocusAll(FIELD)),
        "⌘K asked for {:?}",
        recorder.asked()
    );
    // The panel is up over the empty search: what it offers before anything is typed.
    let page = until(&mut dom, "⌘K", |page| {
        has(page, &format!(">{RECENT}<")) && has(page, "Compose")
    })
    .await;
    assert!(has(&page, "Compose"), "{page}");
    assert!(has(&page, "class=\"spotlight\""), "no panel:\n{page}");
    assert!(
        !has(&page, "class=\"ds-palette"),
        "⌘K still opened the palette"
    );
}

#[tokio::test]
async fn a_press_on_the_toolbars_magnifier_brings_up_the_panel() {
    let recorder = Recorder::default();
    let (mut dom, _built, seen) = window(&recorder);
    click(&mut dom, seen.one("aria-label", BOX_LABEL));
    assert!(
        recorder.asked().contains(&Ask::FocusAll(FIELD)),
        "the magnifier asked for {:?}",
        recorder.asked()
    );
    let page = until(&mut dom, "the magnifier", |page| {
        has(page, &format!("aria-label=\"{LABEL}\""))
    })
    .await;
    assert!(has(&page, "class=\"spotlight\""), "{page}");
}

#[tokio::test]
async fn the_toolbar_holds_only_a_magnifier_and_the_sidebar_has_no_field() {
    let recorder = Recorder::default();
    let (dom, _built, _seen) = window(&recorder);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !has(&page, "class=\"ds-command-pill"),
        "the sidebar still has its pill"
    );
    assert!(!has(&page, "Search or run a command"));
    let side = &page[page.find("class=\"ds-side").expect("the sidebar")..];
    let side = &side[..side.find("class=\"card").unwrap_or(side.len())];
    assert!(!side.contains("<input"), "the sidebar has a field:\n{side}");
    let head = &page[page.find("list-head").expect("the list's toolbar")..];
    let list = head.find("class=\"list\"").expect("the list");
    let head = &head[..list];
    assert!(
        head.contains(&format!("aria-label=\"{BOX_LABEL}\"")),
        "no magnifier in the toolbar:\n{head}"
    );
    // Idle, the toolbar has no field to type in and the panel is not drawn.
    assert!(!head.contains("<input"), "the toolbar has a field:\n{head}");
    assert!(!has(&page, &format!("aria-label=\"{LABEL}\"")));
    assert!(!has(&page, "class=\"spotlight\""));
}

#[tokio::test]
async fn opening_a_mail_keeps_its_search_shown_in_the_toolbar_until_it_is_cleared() {
    let recorder = Recorder::default();
    let (mut dom, _built, _seen) = window(&recorder);
    chord(
        &mut dom,
        "k",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    let seen = drain_seen(&mut dom);
    let field = *seen
        .all("aria-label", LABEL)
        .last()
        .expect("the panel's field");
    type_into(&mut dom, field, "sync");
    // The list shows the mail before the panel does: wait for the panel's top hit, which comes
    // from the search off the thread, so Return opens the mail rather than the first command.
    until(&mut dom, "sync", |page| {
        let words = words(page);
        let Some((_, top)) = words.split_once(TOP) else {
            return false;
        };
        // The top hit's own section, up to whichever section comes next.
        let top = [MAIL, COMMANDS, PLACES]
            .iter()
            .filter_map(|next| top.find(next))
            .min()
            .map_or(top, |end| &top[..end]);
        top.contains("Notes from the sync review")
    })
    .await;
    // The top hit is the mail; Return opens it and the panel goes.
    let seen = chord(&mut dom, "Enter", Modifiers::empty(), field).merge(drain_seen(&mut dom));
    assert!(
        recorder.asked().contains(&Ask::FocusApp),
        "{:?}",
        recorder.asked()
    );
    let page = dioxus_ssr::render(&dom);
    assert!(!has(&page, "class=\"spotlight\""), "the panel stayed up");
    assert!(
        has(&page, "class=\"reader-head\""),
        "no mail opened:\n{page}"
    );
    let head = &page[page.find("list-head").expect("the list's toolbar")..];
    let head = &head[..head.find("class=\"list\"").unwrap_or(head.len())];
    assert!(
        head.contains(">sync<"),
        "the toolbar hides the search:\n{head}"
    );
    assert!(head.contains(&format!("aria-label=\"{CLEAR_LABEL}\"")));
    // Clearing it lists the whole place again and puts the magnifier back.
    click(&mut dom, seen.one("aria-label", CLEAR_LABEL));
    crate::ui::fixtures::drain(&mut dom);
    let page = dioxus_ssr::render(&dom);
    let head = &page[page.find("list-head").expect("the list's toolbar")..];
    let head = &head[..head.find("class=\"list\"").unwrap_or(head.len())];
    assert!(
        !head.contains(&format!("aria-label=\"{CLEAR_LABEL}\"")),
        "{head}"
    );
    assert!(
        head.contains(&format!("aria-label=\"{BOX_LABEL}\"")),
        "{head}"
    );
}
