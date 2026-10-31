//! The search panel as a person drives it: typing brings up the sections, the arrows move
//! across them, Return runs and Tab completes the highlighted row (or an operator being typed),
//! Escape empties then closes the panel, ⌘K or the toolbar's magnifier summons it, the toolbar
//! shows a search the panel left behind, and the sidebar has no field of its own.

use super::pictures::{BarAlone, BarAloneProps, held};
use super::sections::{COMMANDS, PLACES, PLACES_AND_PEOPLE, RECENT};
use super::*;
use crate::ui::fixtures::{chord, dispatching, drain_seen, rebuild_into, type_into, work};
use crate::ui::host::{Ask, Recorder};
use crate::ui::view::{BarListing, BarOpen};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus_core::{ElementId, NoOpMutations, VirtualDom};
use mail_core::SqliteStore;
use mail_core::Store;
use mail_ui_model::copy::menus;
use std::sync::Arc;

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
    .with_root_context(crate::ui::fixtures::client(&store))
    .with_root_context(recorder.host());
    // The panel floats in the root's overlay, drawn the render after it asks.
    let seen = rebuild_into(&mut dom).merge(drain_seen(&mut dom));
    let field = *seen
        .all("aria-label", LABEL)
        .last()
        .expect("the panel's field");
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
pub(super) async fn until(
    dom: &mut VirtualDom,
    text: &str,
    landed: impl Fn(&str) -> bool,
) -> String {
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

/// Whether `page` says `needle`, in its markup or in its words: a row's matched characters are
/// marked, so "Sync now" is drawn as `<span>Sync</span> now`.
pub(super) fn has(page: &str, needle: &str) -> bool {
    page.contains(needle) || words(page).contains(needle)
}

/// `page` without its tags.
pub(super) fn words(page: &str) -> String {
    let mut out = String::with_capacity(page.len());
    let mut inside = false;
    for ch in page.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            ch if !inside => out.push(ch),
            _ => {}
        }
    }
    out
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

/// Typed text is the list's search: the panel names the commands it matches and draws none of
/// the mail, which the list shows.
#[tokio::test]
async fn typing_names_the_commands_and_leaves_the_mail_to_the_list() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    let page = bar
        .typed("sync", |page| {
            has(page, &format!(">{COMMANDS}<")) && has(page, menus::SYNC_NOW)
        })
        .await;
    assert!(
        !has(&page, "Notes from the sync review"),
        "the panel drew mail:\n{page}"
    );
    assert_eq!(search(&bar.dom), "sync");
}

/// Nothing typed and nothing opened yet: the places, then the commands with their keys, and no
/// Recent. Recent is where the person was, not the newest mail, which the list already shows.
#[tokio::test]
async fn an_empty_bar_with_nothing_opened_offers_places_then_commands_and_no_recent() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    let page = until(&mut bar.dom, "", |page| {
        has(page, menus::NEW_MESSAGE) && has(page, &format!(">{COMMANDS}<"))
    })
    .await;
    assert!(
        !has(&page, &format!(">{RECENT}<")),
        "Recent with nothing opened:\n{page}"
    );
    let text = words(&page);
    let places = text.find(PLACES).expect("no Places");
    let commands = text.find(COMMANDS).expect("no Commands");
    assert!(places < commands, "{text}");
    // Where mail is stored is one step away here, as the sidebar leaves it.
    for place in ["Inbox", "Sent", "Drafts", "Spam", "Trash"] {
        assert!(text[places..commands].contains(place), "no {place}: {text}");
    }
}

/// Whether the panel still lists what it lists with nothing typed: every row a typed test waits
/// for is there too, so it waits for this to go.
fn at_rest(page: &str) -> bool {
    has(page, &format!(">{PLACES}<"))
}

#[tokio::test]
async fn return_on_a_command_runs_it_on_the_search_there_was() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("hide sidebar", |page| {
        has(page, menus::HIDE_SIDEBAR) && !at_rest(page)
    })
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
    // One letter that starts a place's name: places and commands both match it.
    let page = bar
        .typed("s", |page| {
            has(page, &format!(">{COMMANDS}<")) && has(page, &format!(">{PLACES_AND_PEOPLE}<"))
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
    // The last row is a command, past the places: the arrows went from one section to the next.
    assert_eq!(active(&bar), Some(rows - 1));
    for _ in 0..rows + 3 {
        bar.key("ArrowUp");
    }
    assert_eq!(active(&bar), Some(0));
    // The arrows typed nothing.
    assert_eq!(search(&bar.dom), "s");
}

#[tokio::test]
async fn tab_completes_the_lit_row_and_an_operator() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("settings", |page| {
        has(page, mail_ui_model::copy::menus::SETTINGS) && !at_rest(page)
    })
    .await;
    bar.key("Tab");
    assert_eq!(search(&bar.dom), menus::SETTINGS);
    assert!(open(&bar.dom).is_some(), "Tab closed the panel");
    // The start of an operator is completed first.
    bar.typed("lunch fr", |page| has(page, "from:")).await;
    bar.key("Tab");
    assert_eq!(search(&bar.dom), "lunch from:");
}

#[tokio::test]
async fn escape_empties_the_field_then_closes_the_panel() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("sync", |page| has(page, menus::SYNC_NOW)).await;
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
    let action = commands::FromTemplate.label();
    bar.typed(action, |page| {
        // The answer to the words typed, not the empty field's: the action leads its section.
        words(page).contains(&format!("{COMMANDS}{action}"))
    })
    .await;
    bar.key("Enter");
    let listing = open(&bar.dom).map(|open| open.listing);
    assert_eq!(listing, Some(BarListing::Templates(String::new())));
    let page = until(&mut bar.dom, "templates", |page| has(page, ">Weekly<")).await;
    assert!(has(&page, ">Templates<"), "{page}");
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

#[tokio::test]
async fn command_k_in_the_panel_selects_what_is_typed() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("sync", |page| has(page, menus::SYNC_NOW)).await;
    chord(&mut bar.dom, "k", crate::ui::fixtures::PRIMARY, bar.field);
    assert!(
        recorder.asked().contains(&Ask::FocusAll(FIELD)),
        "{:?}",
        recorder.asked()
    );
    assert_eq!(search(&bar.dom), "sync");
    assert!(open(&bar.dom).is_some(), "⌘K closed the panel");
}
