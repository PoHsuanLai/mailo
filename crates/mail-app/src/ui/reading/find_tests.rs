//! Ctrl F in the reader, driven through the field the way a person types into it.

use super::Reader;
use super::blocks::{iframe_mounts, reset_iframe_mounts};
use super::tests::{dump_page, html_message, iframe_srcdoc, text_message, thread_of};
use crate::search::Find;
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Scripts, Seen, chord, click, dispatching, rebuild_into, type_into, work,
};
use crate::view::Shell;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use mail_domain::ThreadId;

/// The reader on `thread`, with Ctrl F already open.
#[component]
fn Finding(thread: ThreadId) -> Element {
    let shell = use_signal(|| Shell {
        open: Some(thread),
        find: Some(Find::default()),
        ..Shell::default()
    });
    rsx! { Reader { thread, shell } }
}

struct Open {
    dom: VirtualDom,
    field: ElementId,
    scripts: Scripts,
    _dir: tempfile::TempDir,
}

fn open(parts: &[(&str, Vec<u8>)]) -> Open {
    dispatching();
    let (store, thread, dir) = thread_of(parts);
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new_with_props(Finding, FindingProps { thread })
        .with_root_context(store)
        .with_root_context(scripts.document());
    let seen = rebuild_into(&mut dom);
    let field = seen.one("placeholder", "Find in this thread");
    Open {
        dom,
        field,
        scripts,
        _dir: dir,
    }
}

impl Open {
    fn typed(&mut self, text: &str) -> String {
        type_into(&mut self.dom, self.field, text);
        dioxus_ssr::render(&self.dom)
    }

    fn pressed(&mut self, key: &'static str, modifiers: Modifiers) -> (Seen, String) {
        let seen = chord(&mut self.dom, key, modifiers, self.field);
        (seen, dioxus_ssr::render(&self.dom))
    }
}

/// Two messages, three occurrences of "cursor" between them, one of them capitalised.
fn cursors() -> Vec<(&'static str, Vec<u8>)> {
    let plain = "text/plain; charset=utf-8";
    vec![
        (
            "cursors",
            text_message("cursors", plain, "Cursor one.\r\n\r\nThen cursor two.\r\n"),
        ),
        (
            "cursors",
            text_message("cursors", plain, "A third cursor, and a list.\r\n"),
        ),
    ]
}

/// The find bar's count, read from its own element.
fn count(page: &str) -> Option<String> {
    let (_, rest) = page.split_once(r#"class="find-count mono""#)?;
    let (_, rest) = rest.split_once('>')?;
    rest.split_once('<').map(|(text, _)| text.to_owned())
}

/// The `data-hit` of the current match.
fn now(page: &str) -> Option<String> {
    let (_, rest) = page.split_once(r#"<mark class="hit now" data-hit=""#)?;
    rest.split_once('"').map(|(number, _)| number.to_owned())
}

#[tokio::test]
async fn the_count_follows_enter_both_ways_and_esc_clears_every_mark() {
    let mut at = open(&cursors());
    let page = at.typed("cursor");
    assert_eq!(count(&page).as_deref(), Some("1 of 3"), "{page}");
    assert_eq!(page.matches("<mark").count(), 3, "{page}");
    assert_eq!(now(&page).as_deref(), Some("0"), "{page}");

    let mut seen = Vec::new();
    for _ in 0..3 {
        let (_, page) = at.pressed("Enter", Modifiers::empty());
        seen.push((count(&page), now(&page)));
    }
    let want = [("2 of 3", "1"), ("3 of 3", "2"), ("1 of 3", "0")];
    let want: Vec<_> = want
        .iter()
        .map(|(c, n)| (Some((*c).to_owned()), Some((*n).to_owned())))
        .collect();
    assert_eq!(seen, want, "Enter walks forward and wraps");
    assert!(
        at.scripts
            .all()
            .iter()
            .any(|script| script.contains("mark.hit.now") && script.contains("scrollIntoView")),
        "Enter did not scroll the match into view: {:?}",
        at.scripts.all()
    );

    let (_, page) = at.pressed("Enter", Modifiers::SHIFT);
    assert_eq!(
        count(&page).as_deref(),
        Some("3 of 3"),
        "Shift+Enter wraps back"
    );

    let (_, page) = at.pressed("Escape", Modifiers::empty());
    assert!(!page.contains("<mark"), "Esc left marks behind:\n{page}");
    assert!(
        !page.contains("find-count"),
        "Esc left the bar open:\n{page}"
    );
}

/// The reader on `thread` while the list box holds `cursor`, and no find is open.
#[component]
fn Searching(thread: ThreadId) -> Element {
    let shell = use_signal(|| Shell {
        open: Some(thread),
        search: "from:ada cursor".to_owned(),
        ..Shell::default()
    });
    rsx! { Reader { thread, shell } }
}

#[tokio::test]
async fn the_list_search_marks_the_open_body_without_a_find() {
    let (store, thread, _dir) = thread_of(&cursors());
    let mut dom =
        VirtualDom::new_with_props(Searching, SearchingProps { thread }).with_root_context(store);
    dom.rebuild_in_place();
    let page = dioxus_ssr::render(&dom);
    assert_eq!(page.matches("<mark").count(), 3, "{page}");
    assert!(
        !page.contains("hit now"),
        "a search has no current match:\n{page}"
    );
    assert_eq!(count(&page), None, "a search opened the find bar:\n{page}");
}

#[tokio::test]
async fn a_pattern_finds_and_a_broken_one_says_why_and_marks_nothing() {
    let mut at = open(&cursors());
    let page = at.typed("re:/[Cc]ursor [a-z]+/");
    assert_eq!(count(&page).as_deref(), Some("1 of 2"), "{page}");

    let page = at.typed("re:/(cursor/");
    let own = regex::Regex::new(&String::from("(cursor"))
        .expect_err("an unclosed group")
        .to_string();
    let last = own.lines().last().unwrap_or_default().to_owned();
    assert!(
        page.contains("class=\"find-err mono\"") && page.contains(&last),
        "the regex crate's message ({last:?}) is not on the page:\n{page}"
    );
    assert!(!page.contains("<mark"), "a broken pattern marked:\n{page}");
    assert_eq!(count(&page), None, "a broken pattern still counted");
}

#[tokio::test]
async fn a_match_inside_a_folded_quote_opens_it() {
    let body = "\
Thanks.\r\n\
\r\n\
On Monday Ada wrote:\r\n\
> The list jumps.\r\n\
>\r\n\
> On Sunday Bea wrote:\r\n\
> > Can you look at page two?\r\n";
    let flowed = "text/plain; charset=utf-8; format=flowed";
    let mut at = open(&[("re: page", text_message("re: page", flowed, body))]);
    let folded = at.typed("");
    assert!(
        folded.contains("earlier message"),
        "the chain starts folded:\n{folded}"
    );
    let page = at.typed("page two");
    assert_eq!(count(&page).as_deref(), Some("1 of 2"), "{page}");
    assert_eq!(
        page.matches("<mark").count(),
        2,
        "every match counted is drawn:\n{page}"
    );
}

#[tokio::test]
async fn every_counted_match_is_drawn_across_every_block_kind() {
    // One pattern that hits every word, over bodies with headings, lists, tables, facts, a
    // button, a primary action, quotes and a signature. If the walker named a leaf the
    // renderer does not, the count and the marks on the page would disagree.
    let fixtures = [
        include_str!("../../../../mail-mime/tests/fixtures/block/letter.html"),
        include_str!("../../../../mail-mime/tests/fixtures/block/reply.html"),
        include_str!("../../../../mail-mime/tests/fixtures/block/receipt.html"),
        include_str!("../../../../mail-mime/tests/fixtures/block/newsletter.html"),
    ];
    let parts: Vec<(&str, Vec<u8>)> = fixtures
        .iter()
        .map(|html| ("bodies", html_message("bodies", html)))
        .collect();
    let mut at = open(&parts);
    let page = at.typed("re:/[A-Za-z]+/");
    let counted = count(&page).unwrap_or_else(|| panic!("no count:\n{page}"));
    let total: usize = counted
        .rsplit(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("not a count: {counted}"));
    assert!(
        total > 50,
        "the bodies should have many words, counted {total}"
    );
    assert_eq!(
        page.matches("<mark").count(),
        total,
        "drawn against counted"
    );
}

#[tokio::test]
async fn a_find_never_touches_the_original_frame() {
    reset_iframe_mounts();
    let body = include_str!("../../../../mail-mime/tests/fixtures/block/newsletter.html");
    let mut at = open(&[("issue", html_message("issue", body))]);
    let before = at.typed("");
    let srcdoc = iframe_srcdoc(&before);
    let mounted = iframe_mounts();
    assert!(mounted >= 1, "a laid-out message mounts its frame");

    let after = at.typed("issue");
    assert!(
        after.contains("<mark"),
        "the blocks were not marked:\n{after}"
    );
    assert_eq!(
        iframe_srcdoc(&after),
        srcdoc,
        "the frame's document changed"
    );
    assert_eq!(iframe_mounts(), mounted, "the frame was remounted");
    let frame = after
        .split_once("<iframe")
        .and_then(|(_, rest)| rest.split_once('>'))
        .map(|(tag, _)| tag.to_owned())
        .unwrap_or_default();
    assert!(
        !frame.contains("mark"),
        "the frame tag carries a mark: {frame}"
    );
}

#[tokio::test]
async fn ctrl_f_opens_the_field_on_the_open_thread_and_esc_closes_it() {
    dispatching();
    let built = work();
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store)
        .with_root_context(built.dirs)
        .with_root_context(scripts.document());
    let seen = rebuild_into(&mut dom);
    let shell = ElementId(INSIDE_THE_SHELL as usize);

    // Nothing open: there is no thread to find in, so the chord goes to the list's box.
    chord(&mut dom, "f", Modifiers::CONTROL, shell);
    assert!(
        scripts.all().iter().any(|s| s.contains(".search input")),
        "Ctrl F with nothing open did not reach the search box: {:?}",
        scripts.all()
    );
    assert!(!dioxus_ssr::render(&dom).contains("Find in this thread"));

    let row = seen.one(
        "aria-label",
        "Open Re: UIDL stability across a UIDVALIDITY change",
    );
    click(&mut dom, row);
    chord(&mut dom, "f", Modifiers::CONTROL, shell);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Find in this thread"),
        "Ctrl F opened no field:\n{page}"
    );
    assert!(
        scripts.all().iter().any(|s| s.contains("focusFind")),
        "the field was not focused: {:?}",
        scripts.all()
    );

    chord(&mut dom, "Escape", Modifiers::empty(), shell);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("Find in this thread"),
        "Esc left the field open:\n{page}"
    );
    assert!(
        page.contains("UIDL stability"),
        "Esc closed the thread along with the find:\n{page}"
    );
}

/// The reader mid-find, for a screenshot in both themes.
///
/// ```text
/// cargo test -p mail-app -- --ignored render_a_find_to_a_file
/// ```
#[tokio::test]
#[ignore = "writes target/find.html for a screenshot; run with --ignored"]
async fn render_a_find_to_a_file() {
    let reply = include_str!("../../../../mail-mime/tests/fixtures/block/reply.html");
    let mut parts = cursors();
    parts.push(("cursors", html_message("cursors", reply)));
    let mut at = open(&parts);
    at.typed("cursor");
    let (_, page) = at.pressed("Enter", Modifiers::empty());
    dump_page(
        "find",
        &format!("<div class=\"app\"><div class=\"reader\">{page}</div></div>"),
    );
}
