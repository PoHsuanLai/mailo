//! The list box through the pipeline, typed into the real window over the reference fixture.

use super::row_hit;
use crate::search::list_highlight;
use crate::ui::app::App;
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
    let search = seen.one("placeholder", "Search all mail");
    Window {
        dom,
        search,
        _built: (root, dana),
    }
}

impl Window {
    /// Type `text` as the box's whole value, and let the off-thread list land.
    async fn typed(&mut self, text: &str) -> String {
        type_into(&mut self.dom, self.search, text);
        for _ in 0..16 {
            let waited = tokio::time::timeout(
                std::time::Duration::from_millis(40),
                self.dom.wait_for_work(),
            )
            .await;
            if waited.is_err() {
                break;
            }
            self.dom.render_immediate(&mut NoOpMutations);
        }
        self.dom.render_immediate(&mut NoOpMutations);
        dioxus_ssr::render(&self.dom)
    }
}

/// Each row's subject cell, markup and all, in list order.
fn subjects(page: &str) -> Vec<String> {
    page.split(r#"<div class="row-sub">"#)
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

#[tokio::test]
async fn half_a_word_puts_its_thread_first_with_the_typed_part_marked() {
    let mut window = window();
    let page = window.typed("uidval").await;
    let rows = subjects(&page);
    assert_eq!(
        rows.first().map(String::as_str),
        Some(
            r#"Re: UIDL stability across a <mark class="hit" data-hit="0">UIDVAL</mark>IDITY change"#
        ),
        "rows: {rows:?}"
    );
}

#[tokio::test]
async fn clearing_the_box_gives_the_place_back() {
    let mut window = window();
    let place = subjects(&window.typed("").await);
    assert!(
        place.len() > 2,
        "the Work inbox has more than two threads: {place:?}"
    );
    let searched = subjects(&window.typed("cursors").await);
    assert_ne!(searched, place, "the search changed nothing");
    let back = subjects(&window.typed("").await);
    assert_eq!(back, place, "clearing the box did not restore the place");
    assert!(
        !back.iter().any(|row| row.contains("<mark")),
        "marks outlived the search"
    );
}

#[tokio::test]
async fn a_bare_pattern_runs_over_this_page_and_says_so() {
    let mut window = window();
    let page = window.typed("re:/^Re: /").await;
    assert_eq!(
        subjects(&page),
        vec![
            r#"<mark class="hit" data-hit="0">Re: </mark>UIDL stability across a UIDVALIDITY change"#,
            r#"<mark class="hit" data-hit="0">Re: </mark>Re: Keyset cursors, not offsets"#,
        ],
    );
    assert!(
        notes(&page).contains(&"pattern over this page".to_owned()),
        "the bar did not say where the pattern ran: {:?}",
        notes(&page)
    );

    // Narrowed by a word, it is a search again, and the bar has nothing to add.
    let page = window.typed("re:/^Re: / keyset").await;
    assert_eq!(subjects(&page).len(), 1, "{:?}", subjects(&page));
    assert!(notes(&page).is_empty(), "{:?}", notes(&page));
}

#[tokio::test]
async fn a_broken_pattern_is_the_regex_message_in_the_bar_and_no_rows() {
    let mut window = window();
    let page = window.typed("re:/(/").await;
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
#[tokio::test]
#[ignore = "writes target/search.html for a screenshot; run with --ignored"]
async fn render_a_search_to_a_file() {
    let mut window = window();
    let page = window.typed("cursor").await;
    dump("search", &page);
}
