//! The list box through the pipeline, typed into the real window over the reference fixture.
//!
//! Every test here runs on a paused clock and drives it: a keystroke is searched for only once
//! the box has been still for [`QUIET`], and the list lands from a blocking thread. So a test
//! types, moves the clock past the quiet period, and then waits for the very condition it goes
//! on to assert. It never counts polls or sleeps: a fixed number of polls is a guess about how
//! fast the blocking thread is, and under a loaded `cargo test --workspace` the guess was wrong.

use super::row_hit;
use crate::search::list_highlight;
use crate::ui::app::App;
use crate::ui::debounce::QUIET;
use crate::ui::fixtures::{Work, dispatching, dump, rebuild_into, type_into, work};
use chrono::Utc;
use dioxus::prelude::*;
use dioxus_core::{ElementId, NoOpMutations, VirtualDom};
use mail_domain::*;

/// The window over the Work Space, and its search box.
struct Window {
    dom: VirtualDom,
    search: ElementId,
    _built: (tempfile::TempDir, ThreadId),
}

fn window() -> Window {
    dispatching();
    let Work {
        store,
        root,
        dirs,
        dana,
        ..
    } = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(store)
        .with_root_context(dirs);
    let seen = rebuild_into(&mut dom);
    let search = seen.one("aria-placeholder", "Search all mail");
    Window {
        dom,
        search,
        _built: (root, dana),
    }
}

impl Window {
    /// Type `text` as the box's whole value, let the box go still, and wait until `landed`
    /// holds for the page. Returns that page.
    async fn typed(&mut self, text: &str, landed: impl Fn(&str) -> bool) -> String {
        type_into(&mut self.dom, self.search, text);
        tokio::time::advance(QUIET).await;
        self.until(text, landed).await
    }

    /// Render until `landed` holds, doing the window's pending work in between.
    ///
    /// Bounded in the paused clock's time, not the wall's: while the list is being fetched on
    /// its blocking thread the clock cannot move (tokio holds it for `spawn_blocking`), so the
    /// bound is only reached if the window goes idle without ever showing the condition.
    async fn until(&mut self, text: &str, landed: impl Fn(&str) -> bool) -> String {
        let start = tokio::time::Instant::now();
        loop {
            self.dom.render_immediate(&mut NoOpMutations);
            let page = dioxus_ssr::render(&self.dom);
            if landed(&page) {
                return page;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "after typing {text:?} the list never showed what the test waits for: {:?} {:?}",
                subjects(&page),
                notes(&page)
            );
            self.dom.wait_for_work().await;
        }
    }
}

/// Each row's subject cell, markup and all, in list order.
fn subjects(page: &str) -> Vec<String> {
    page.split(r#"<div class="ds-row-sub ds-truncate">"#)
        .skip(1)
        .filter_map(|rest| rest.split_once("</div>"))
        .map(|(subject, _)| subject.to_owned())
        .collect()
}

/// The list bar's notes: sync state and search scope alike.
fn notes(page: &str) -> Vec<String> {
    let bar = page
        .split_once(r#"class="list-bar""#)
        .and_then(|(_, rest)| rest.split_once(r#"class="bar-tools""#))
        .map(|(bar, _)| bar)
        .unwrap_or_default();
    bar.split("class=\"status")
        .skip(1)
        .filter_map(|rest| rest.split_once('>'))
        .filter_map(|(_, rest)| rest.split_once('<'))
        .map(|(text, _)| text.to_owned())
        .collect()
}

/// The strip's heading, as markup: the words alone are also in the stylesheet's comments.
const TOP_RESULTS: &str = r#"<li class="list-top-h">Top results</li>"#;
const NEWEST_FIRST: &str = r#"<li class="list-top-h">Newest first</li>"#;

const UIDVAL: &str =
    r#"Re: UIDL stability across a <mark class="ds-mark">UIDVAL</mark>IDITY change"#;

#[tokio::test(start_paused = true)]
async fn half_a_word_puts_its_thread_first_with_the_typed_part_marked() {
    let mut window = window();
    let page = window
        .typed("uidval", |page| {
            subjects(page).first().map(String::as_str) == Some(UIDVAL)
        })
        .await;
    assert_eq!(
        subjects(&page),
        vec![UIDVAL.to_owned()],
        "one match, and no strip repeating it"
    );
    assert!(!page.contains(TOP_RESULTS), "a strip of the one row");
}

#[tokio::test(start_paused = true)]
async fn a_keystroke_is_searched_for_only_once_the_box_is_still() {
    let mut window = window();
    let place = subjects(&dioxus_ssr::render(&window.dom));
    type_into(&mut window.dom, window.search, "uidval");
    window.dom.render_immediate(&mut NoOpMutations);
    assert_eq!(
        subjects(&dioxus_ssr::render(&window.dom)),
        place,
        "the list searched on the keystroke itself"
    );
    let page = window
        .typed("uidval", |page| {
            subjects(page).first().map(String::as_str) == Some(UIDVAL)
        })
        .await;
    assert_ne!(subjects(&page), place);
}

#[tokio::test(start_paused = true)]
async fn a_word_lists_newest_first_under_its_top_results() {
    let mut window = window();
    let page = window
        .typed("cursors", |page| page.contains(TOP_RESULTS))
        .await;
    let (strip, list) = page
        .split_once(NEWEST_FIRST)
        .expect("the strip has a heading for the list under it");
    let plain = |rows: Vec<String>| -> Vec<String> {
        rows.into_iter()
            .map(|row| {
                row.replace(r#"<mark class="ds-mark">"#, "")
                    .replace("</mark>", "")
            })
            .collect()
    };
    assert_eq!(
        plain(subjects(list)),
        vec![
            "Notes from the sync review".to_owned(),
            "Re: Re: Keyset cursors, not offsets".to_owned(),
        ],
        "the matches, newest first"
    );
    let strip = subjects(strip);
    assert_eq!(
        strip.first().map(String::as_str),
        Some(r#"Re: Re: Keyset <mark class="ds-mark">cursors</mark>, not offsets"#),
        "the subject hit leads the strip, marked: {strip:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn clearing_the_box_gives_the_place_back() {
    let mut window = window();
    let place = subjects(&dioxus_ssr::render(&window.dom));
    assert!(
        place.len() > 2,
        "the Work inbox has more than two threads: {place:?}"
    );
    let searched = subjects(&window.typed("cursors", |page| page.contains("<mark")).await);
    assert_ne!(searched, place, "the search changed nothing");
    let back = subjects(&window.typed("", |page| subjects(page) == place).await);
    assert_eq!(back, place, "clearing the box did not restore the place");
    assert!(
        !back.iter().any(|row| row.contains("<mark")),
        "marks outlived the search"
    );
}

#[tokio::test(start_paused = true)]
async fn a_bare_pattern_runs_over_this_page_and_says_so() {
    let mut window = window();
    let over_page = |page: &str| notes(page).contains(&"pattern over this page".to_owned());
    let page = window.typed("re:/^Re: /", over_page).await;
    assert_eq!(
        subjects(&page),
        vec![
            r#"<mark class="ds-mark">Re:</mark> UIDL stability across a UIDVALIDITY change"#,
            r#"<mark class="ds-mark">Re:</mark> Re: Keyset cursors, not offsets"#,
        ],
    );
    assert!(
        notes(&page).contains(&"pattern over this page".to_owned()),
        "the bar did not say where the pattern ran: {:?}",
        notes(&page)
    );

    // Narrowed by a word, it is a search again, and the bar has nothing to add.
    let page = window
        .typed("re:/^Re: / keyset", |page| !over_page(page))
        .await;
    assert_eq!(subjects(&page).len(), 1, "{:?}", subjects(&page));
    assert!(notes(&page).is_empty(), "{:?}", notes(&page));
}

#[tokio::test(start_paused = true)]
async fn a_broken_pattern_is_the_regex_message_in_the_bar_and_no_rows() {
    let mut window = window();
    let page = window
        .typed("re:/(/", |page| page.contains(r#"class="status bad""#))
        .await;
    let own = regex::Regex::new(&String::from("("))
        .expect_err("an unclosed group")
        .to_string();
    assert!(subjects(&page).is_empty(), "{:?}", subjects(&page));
    assert!(
        page.contains(r#"class="status bad""#) && notes(&page).contains(&own),
        "the bar did not carry {own:?}: {:?}",
        notes(&page)
    );
}

fn summary(subject: &str, snippet: &str) -> ThreadSummary {
    ThreadSummary {
        id: ThreadId::generate(),
        account: AccountId::generate(),
        subject: subject.to_owned(),
        snippet: snippet.to_owned(),
        from: Address {
            name: None,
            email: "a@example.test".to_owned(),
        },
        participants: vec![],
        recipients: vec![],
        last_date: Utc::now(),
        message_count: 1,
        read: ReadState::Read,
        star: Star::Unstarred,
        mailboxes: MailboxSet::only(MailboxRole::Inbox),
        labels: vec![],
        attachments: Attachments::None,
        snooze: Snooze::Inactive,
        pin: Pin::Unpinned,
    }
}

#[test]
fn row_marks_sit_on_char_boundaries_after_cjk_and_emoji() {
    struct Case {
        name: &'static str,
        typed: &'static str,
        subject: &'static str,
        want_subject: &'static [&'static str],
        want_snippet: &'static [&'static str],
    }
    let cases = [
        Case {
            name: "cjk",
            typed: "郵件",
            subject: "校園郵件通知 😀",
            want_subject: &["郵件"],
            want_snippet: &[],
        },
        Case {
            name: "emoji before",
            typed: "valid",
            subject: "😀valid 🎉",
            want_subject: &["valid"],
            want_snippet: &[],
        },
        Case {
            name: "snippet cut around its match",
            typed: "wrinkle",
            subject: "plain subject",
            want_subject: &[],
            want_snippet: &["wrinkle"],
        },
    ];
    let filler = "😀".repeat(40);
    for Case {
        name,
        typed,
        subject,
        want_subject,
        want_snippet,
    } in cases
    {
        let highlight = list_highlight(typed, &Utc, &|_| Vec::new()).expect("no pattern");
        let snippet = format!("{filler} one wrinkle: two threads");
        let hit = row_hit(&summary(subject, &snippet), &highlight).expect("a search is active");
        let marked: Vec<&str> = hit.subject.iter().map(|r| &subject[r.clone()]).collect();
        assert_eq!(marked, *want_subject, "{name}: subject");
        let cut: Vec<&str> = hit
            .snippet_marks
            .iter()
            .map(|r| &hit.snippet[r.clone()])
            .collect();
        assert_eq!(cut, *want_snippet, "{name}: snippet");
        if !want_snippet.is_empty() {
            assert!(
                hit.snippet.starts_with('…') && hit.snippet.contains("wrinkle"),
                "{name}: the snippet was not cut around the match: {}",
                hit.snippet
            );
        }
    }
    assert_eq!(
        row_hit(&summary("a", "b"), &Default::default()),
        None,
        "no search, no marks"
    );
}

/// The list mid-search, for a screenshot in both themes.
///
/// ```text
/// cargo test -p mail-app -- --ignored render_a_search_to_a_file
/// ```
#[tokio::test(start_paused = true)]
#[ignore = "writes target/search.html for a screenshot; run with --ignored"]
async fn render_a_search_to_a_file() {
    let mut window = window();
    let page = window
        .typed("cursor", |page| page.contains(TOP_RESULTS))
        .await;
    dump("search", &page);
}
