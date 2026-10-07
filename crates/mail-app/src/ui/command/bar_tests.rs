//! The search bar as a person drives it: typing brings up the sections, the arrows move across
//! them, Return runs and Tab completes the highlighted row, Escape empties then leaves the
//! field, ⌘K summons it, and the sidebar no longer has a field of its own.

use super::pictures::{BarAlone, BarAloneProps, held};
use super::sections::{COMMANDS, MAIL, RECENT, TOP};
use super::*;
use crate::ui::app::App;
use crate::ui::fixtures::{INSIDE_THE_SHELL, chord, dispatching, rebuild_into, type_into, work};
use crate::ui::host::{Ask, Recorder};
use crate::ui::view::{BarListing, BarOpen};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus_core::{ElementId, NoOpMutations, VirtualDom};
use mail_store::Store;

/// The bar alone over the reference fixture, its field, and the fixture kept alive.
struct Alone<K = crate::ui::fixtures::Work> {
    dom: VirtualDom,
    field: ElementId,
    _kept: K,
}

fn alone(recorder: &Recorder) -> Alone {
    let built = work();
    let store = built.store.clone();
    alone_over(recorder, store, built)
}

/// The bar alone over `store`, holding `kept` for as long as the test runs.
fn alone_over<K: 'static>(recorder: &Recorder, store: Arc<SqliteStore>, kept: K) -> Alone<K> {
    dispatching();
    let mut dom = VirtualDom::new_with_props(
        BarAlone,
        BarAloneProps {
            typed: String::new(),
        },
    )
    .with_root_context(store)
    .with_root_context(recorder.host());
    let seen = rebuild_into(&mut dom);
    let field = *seen
        .all("aria-label", LABEL)
        .last()
        .expect("the bar's field");
    Alone {
        dom,
        field,
        _kept: kept,
    }
}

impl<K> Alone<K> {
    /// Type `text` as the field's whole value and wait until the panel answers it.
    async fn typed(&mut self, text: &str, landed: impl Fn(&str) -> bool) -> String {
        type_into(&mut self.dom, self.field, text);
        until(&mut self.dom, text, landed).await
    }

    fn key(&mut self, name: &'static str) {
        chord(&mut self.dom, name, Modifiers::empty(), self.field);
        self.dom.render_immediate(&mut NoOpMutations);
    }
}

/// Render until `landed` holds for the page, doing the window's work in between. The search
/// waits for the field to be still, then runs off the thread that draws.
async fn until(dom: &mut VirtualDom, text: &str, landed: impl Fn(&str) -> bool) -> String {
    let start = std::time::Instant::now();
    loop {
        dom.render_immediate(&mut NoOpMutations);
        let page = dioxus_ssr::render(dom);
        if landed(&page) {
            return page;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "after {text:?} the panel never showed what the test waits for:\n{page}"
        );
        tokio::time::timeout(std::time::Duration::from_millis(200), dom.wait_for_work())
            .await
            .ok();
    }
}

fn open(dom: &VirtualDom) -> Option<BarOpen> {
    dom.in_runtime(|| match &held().shell.peek().bar {
        Bar::Open(open) => Some(open.clone()),
        Bar::Closed => None,
    })
}

fn search(dom: &VirtualDom) -> String {
    dom.in_runtime(|| held().shell.peek().search.clone())
}

#[tokio::test]
async fn typing_shows_the_mail_and_the_commands_it_names() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    let page = bar.typed("sync", |page| page.contains("Sync now")).await;
    for title in [COMMANDS, "Notes from the sync review", "Sync now"] {
        assert!(page.contains(title), "no {title:?} for sync:\n{page}");
    }
    assert!(
        page.contains(&format!(">{MAIL}<")) || page.contains(&format!(">{TOP}<")),
        "no Mail section for sync:\n{page}"
    );
    // What is typed is the list's search too.
    assert_eq!(search(&bar.dom), "sync");
}

#[tokio::test]
async fn an_empty_bar_offers_recent_mail_and_commands() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    let page = until(&mut bar.dom, "", |page| page.contains("Compose")).await;
    assert!(page.contains(&format!(">{RECENT}<")), "no Recent:\n{page}");
    assert!(
        page.contains(&format!(">{COMMANDS}<")),
        "no Commands:\n{page}"
    );
}

#[tokio::test]
async fn return_on_a_command_runs_it_on_the_search_there_was() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("hide sidebar", |page| page.contains("Hide sidebar"))
        .await;
    bar.key("Enter");
    assert!(
        bar.dom.in_runtime(|| *held().side_hidden.peek()),
        "Return on Hide sidebar left the sidebar"
    );
    // The panel is gone, the keyboard is the list's, and the command's name was never a search.
    assert_eq!(open(&bar.dom), None);
    assert_eq!(search(&bar.dom), "");
    assert!(
        recorder.asked().contains(&Ask::FocusApp),
        "{:?}",
        recorder.asked()
    );
}

#[tokio::test]
async fn the_arrows_move_across_the_sections_and_stop_at_the_ends() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    // One letter: mail, commands and places all match it.
    let page = bar
        .typed("e", |page| {
            page.contains(&format!(">{COMMANDS}<")) && page.contains(&format!(">{TOP}<"))
        })
        .await;
    let rows = page.matches("role=\"menuitem\"").count();
    assert!(rows >= 3, "too few rows to cross a section: {rows}\n{page}");
    let active = |bar: &Alone| open(&bar.dom).map(|open| open.active);
    assert_eq!(active(&bar), Some(0));
    bar.key("ArrowDown");
    bar.key("ArrowDown");
    assert_eq!(active(&bar), Some(2));
    bar.key("ArrowUp");
    assert_eq!(active(&bar), Some(1));
    for _ in 0..rows + 3 {
        bar.key("ArrowDown");
    }
    // The last row is a command, past the mail: the arrows went from one section to the next.
    assert_eq!(active(&bar), Some(rows - 1));
    for _ in 0..rows + 3 {
        bar.key("ArrowUp");
    }
    assert_eq!(active(&bar), Some(0));
    // The arrows typed nothing.
    assert_eq!(search(&bar.dom), "e");
}

#[tokio::test]
async fn tab_completes_the_top_row() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("settings", |page| page.contains("Settings…"))
        .await;
    bar.key("Tab");
    assert_eq!(search(&bar.dom), "Settings…");
    assert!(open(&bar.dom).is_some(), "Tab closed the panel");
}

#[tokio::test]
async fn escape_empties_the_field_then_leaves_it() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("sync", |page| page.contains("Sync now")).await;
    bar.key("Escape");
    assert_eq!(search(&bar.dom), "");
    assert!(
        open(&bar.dom).is_some(),
        "the first Escape closed the panel"
    );
    assert!(!recorder.asked().contains(&Ask::FocusApp));
    bar.key("Escape");
    assert_eq!(open(&bar.dom), None);
    assert!(
        recorder.asked().contains(&Ask::FocusApp),
        "the second Escape left the keyboard in the field: {:?}",
        recorder.asked()
    );
}

#[tokio::test]
async fn new_from_template_lists_the_templates_in_the_panel_and_starts_one() {
    let recorder = Recorder::default();
    let (store, dir) = crate::ui::fixtures::seeded();
    let mut bar = alone_over(&recorder, store.clone(), dir);
    let account = crate::ui::fixtures::acct_account();
    let draft = mail_core::compose::draft_new(
        &store,
        account.clone(),
        &[],
        "Weekly notes",
        "Here is what moved this week.",
        chrono::Utc::now(),
    )
    .unwrap_or_else(|why| panic!("a draft: {why}"));
    mail_core::template::save(&store, draft.id, "Weekly", chrono::Utc::now())
        .unwrap_or_else(|why| panic!("a template: {why}"));
    // The row, not the field's own value, which holds the same words.
    bar.typed(templates::ACTION, |page| {
        page.contains(&format!(">{}<", templates::ACTION))
    })
    .await;
    bar.key("Enter");
    let listing = open(&bar.dom).map(|open| open.listing);
    assert_eq!(listing, Some(BarListing::Templates(String::new())));
    let page = until(&mut bar.dom, "templates", |page| page.contains(">Weekly<")).await;
    assert!(page.contains(">Templates<"), "{page}");
    // The list's search is what it was before the action's name was typed.
    assert_eq!(search(&bar.dom), "");
    bar.key("Enter");
    let opened = bar
        .dom
        .in_runtime(|| {
            held()
                .shell
                .peek()
                .composing
                .as_ref()
                .map(|open| open.draft)
        })
        .unwrap_or_else(|| panic!("the draft did not open as a page"));
    let started = store.draft(opened).unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(started.subject, "Weekly notes");
    assert_eq!(open(&bar.dom), None);
}

/// The window over the reference fixture, with a recorder for what it asks of its host.
fn window(recorder: &Recorder) -> (VirtualDom, crate::ui::fixtures::Work) {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(recorder.host());
    rebuild_into(&mut dom);
    (dom, built)
}

#[tokio::test]
async fn command_k_puts_the_keyboard_in_the_bar_with_its_text_selected() {
    let recorder = Recorder::default();
    let (mut dom, _built) = window(&recorder);
    chord(
        &mut dom,
        "k",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    assert!(
        recorder.asked().contains(&Ask::FocusAll(".search input")),
        "⌘K asked for {:?}",
        recorder.asked()
    );
    // The panel is up over the empty search: what the bar offers before anything is typed.
    let page = until(&mut dom, "⌘K", |page| {
        page.contains(&format!(">{RECENT}<"))
    })
    .await;
    assert!(page.contains("Compose"), "{page}");
    assert!(
        !page.contains("class=\"ds-palette"),
        "⌘K still opened the palette"
    );
}

#[tokio::test]
async fn the_bar_is_in_the_lists_toolbar_and_the_sidebar_has_no_field() {
    let recorder = Recorder::default();
    let (dom, _built) = window(&recorder);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("class=\"ds-command-pill"),
        "the sidebar still has its pill"
    );
    assert!(!page.contains("Search or run a command"));
    let side = &page[page.find("class=\"ds-side").expect("the sidebar")..];
    let side = &side[..side.find("class=\"card").unwrap_or(side.len())];
    assert!(!side.contains("<input"), "the sidebar has a field:\n{side}");
    let head = &page[page.find("list-head").expect("the list's toolbar")..];
    let bar = head
        .find(&format!("aria-label=\"{LABEL}\""))
        .expect("no bar in the toolbar");
    let list = head.find("class=\"list\"").expect("the list");
    assert!(bar < list, "the bar is not above the list");
}
