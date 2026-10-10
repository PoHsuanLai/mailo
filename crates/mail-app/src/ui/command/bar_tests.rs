//! The search panel as a person drives it: typing brings up the sections, the arrows move
//! across them, Return runs and Tab completes the highlighted row, Escape empties then closes
//! the panel, ⌘K or the toolbar's magnifier summons it, the toolbar shows a search the panel
//! left behind, and the sidebar has no field of its own.

use super::pictures::{BarAlone, BarAloneProps, held};
use super::sections::{COMMANDS, MAIL, RECENT, TOP};
use super::*;
use crate::ui::fixtures::{chord, dispatching, drain_seen, rebuild_into, type_into, work};
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

#[tokio::test]
async fn typing_shows_the_mail_and_the_commands_it_names() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    // The commands answer at once; the mail comes later, from the search off the thread. Wait
    // for all of what is asserted, not the first of it to land.
    let titles = [COMMANDS, "Notes from the sync review", "Sync now"];
    let mail = |page: &str| has(page, &format!(">{MAIL}<")) || has(page, &format!(">{TOP}<"));
    let page = bar
        .typed("sync", |page| {
            titles.iter().all(|title| has(page, title)) && mail(page)
        })
        .await;
    for title in titles {
        assert!(has(&page, title), "no {title:?} for sync:\n{page}");
    }
    assert!(mail(&page), "no Mail section for sync:\n{page}");
    // What is typed is the list's search too.
    assert_eq!(search(&bar.dom), "sync");
}

#[tokio::test]
async fn an_empty_bar_offers_recent_mail_and_commands() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    let page = until(&mut bar.dom, "", |page| {
        has(page, "Compose")
            && has(page, &format!(">{RECENT}<"))
            && has(page, &format!(">{COMMANDS}<"))
    })
    .await;
    assert!(has(&page, &format!(">{RECENT}<")), "no Recent:\n{page}");
    assert!(has(&page, &format!(">{COMMANDS}<")), "no Commands:\n{page}");
}

#[tokio::test]
async fn return_on_a_command_runs_it_on_the_search_there_was() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("hide sidebar", |page| has(page, "Hide sidebar"))
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
            has(page, &format!(">{COMMANDS}<")) && has(page, &format!(">{TOP}<"))
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
    bar.typed("settings", |page| has(page, "Settings…")).await;
    bar.key("Tab");
    assert_eq!(search(&bar.dom), "Settings…");
    assert!(open(&bar.dom).is_some(), "Tab closed the panel");
}

#[tokio::test]
async fn escape_empties_the_field_then_closes_the_panel() {
    let recorder = Recorder::default();
    let mut bar = alone(&recorder);
    bar.typed("sync", |page| has(page, "Sync now")).await;
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
        // The answer to the words typed, not the empty field's: the action is the top hit.
        words(page).contains(&format!("{TOP}{}", templates::ACTION))
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
    bar.typed("sync", |page| has(page, "Sync now")).await;
    chord(&mut bar.dom, "k", crate::ui::fixtures::PRIMARY, bar.field);
    assert!(
        recorder.asked().contains(&Ask::FocusAll(FIELD)),
        "{:?}",
        recorder.asked()
    );
    assert_eq!(search(&bar.dom), "sync");
    assert!(open(&bar.dom).is_some(), "⌘K closed the panel");
}
