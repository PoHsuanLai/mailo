//! The list pane, driven through the real `App`.

use super::super::app::App;
use crate::ui::fixtures::{acct_account, dispatching, seeded};
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use searching_in_the_window::listed;
use std::sync::Arc;

/// Render until `landed` holds for the page, doing the tree's pending work in between.
///
/// The list is computed on a blocking thread, and a search waits for the box to
/// be still first, so one render after a keystroke shows what was on screen *before* it. That
/// is the right behaviour in a window, where a blank pane between keystrokes is worse than a
/// stale one, and the wrong thing to assert against. So the test waits for the condition it
/// asserts, never for a number of polls: counting polls guessed how fast the blocking thread
/// is, and under a loaded `cargo test --workspace` the guess was wrong.
///
/// The tests run on a paused clock. It cannot move while the list is fetched on its blocking
/// thread (tokio holds it for `spawn_blocking`), so the bound below is only reached when the
/// tree goes idle without ever showing the condition.
async fn until(dom: &mut VirtualDom, landed: impl Fn(&str) -> bool) -> String {
    let start = tokio::time::Instant::now();
    loop {
        dom.render_immediate(&mut NoOpMutations);
        let page = dioxus_ssr::render(dom);
        if landed(&page) {
            return page;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "the list never showed what the test waits for: {:?}",
            listed(&page)
        );
        dom.wait_for_work().await;
    }
}

/// Typing into the real window's search box, and reading what the list pane then shows.
///
/// `label:` is the one search term whose answer depends on state the component loads from
/// the store. A test that sets `Shell::search` directly would build that state itself and
/// prove nothing about whether `App` ever does — which is exactly how this shipped broken:
/// the window passed a resolver that knew no label names, so `label:travel` quietly became a
/// full-text search for the literal string while the same query worked in the terminal.
mod searching_in_the_window {
    use super::*;

    fn labelled() -> (Arc<SqliteStore>, tempfile::TempDir) {
        let (store, dir) = seeded();
        // The way a Gmail sync reports it: the complete label set for one remote message.
        // Writing `labels` and `message_labels` by hand looked equivalent and was not —
        // `Filter::HasLabel` reads `thread_summary.labels`, a materialized union that only
        // the ingest path rewrites, so the rows were there and no search could see them.
        store
            .ingest(
                acct_account(),
                Ingest {
                    mailbox: MailboxRef {
                        account: acct_account(),
                        path: "INBOX".to_owned(),
                    },
                    validity: UidValidity::Same,
                    cursor: None,
                    messages: vec![],
                    flags: vec![],
                    flag_colors: Vec::new(),
                    labels: vec![],
                    label_names: vec![(
                        RemoteRef::Pop {
                            uidl: "u1".to_owned(),
                        },
                        vec!["travel".to_owned()],
                    )],
                    provider_threads: Vec::new(),
                    gone: vec![],
                },
            )
            .unwrap();
        (store, dir)
    }

    /// Mount `App`, type `typed` into its search panel, let the field go still, and return the
    /// page once `landed` holds for it.
    async fn typing(store: Arc<SqliteStore>, typed: &str, landed: impl Fn(&str) -> bool) -> String {
        dispatching();
        let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
        dom.rebuild_in_place();
        // Let the mount-time effects run: the label index is one of them, and the whole
        // question is whether it is there by the time someone types.
        tokio::time::timeout(std::time::Duration::from_millis(500), dom.wait_for_work())
            .await
            .ok();
        dom.render_immediate(&mut NoOpMutations);
        // ⌘K brings up the search panel, whose field is the list's search. It floats in the
        // root's overlay, drawn the renders after it asks.
        crate::ui::fixtures::chord(
            &mut dom,
            "k",
            crate::ui::fixtures::PRIMARY,
            dioxus_core::ElementId(crate::ui::fixtures::INSIDE_THE_SHELL as usize),
        );
        let drawn = crate::ui::fixtures::drain_seen(&mut dom);
        let field = *drawn
            .all("aria-label", crate::ui::command::LABEL)
            .last()
            .expect("the search panel's field");
        crate::ui::fixtures::type_into(&mut dom, field, typed);
        tokio::time::advance(mail_ui_kit::debounce::QUIET).await;
        // The list may already look like the answer before the search lands (the Inbox can hold
        // just the one conversation), so wait for the panel to show what was typed as well: the
        // whole state the tests go on to read, not the first part of it to arrive.
        let shown = format!(r#"value="{typed}""#);
        until(&mut dom, |page| {
            page.contains(&format!(r#"aria-label="{}""#, crate::ui::command::LABEL))
                && page.contains(&shown)
                && landed(page)
        })
        .await
    }

    /// The subjects the list pane is showing.
    ///
    /// Read from the subject cells rather than searched for in the page: the stylesheet is
    /// in the markup, and `page.contains("hi")` is true of `white-space` and `this`. That is
    /// "Substrings are not tokens" (`CONTRIBUTING.md`), in a test of its own making.
    pub(super) fn listed(page: &str) -> Vec<String> {
        // A row that has left is still drawn until its exit settles, under
        // `data-presence="leaving"`: it is not one of the rows the list is showing.
        page.split(r#"class="ds-list-item""#)
            .skip(1)
            .filter(|item| {
                !item
                    .trim_start()
                    .starts_with(r#"role="none" data-presence="leaving""#)
            })
            .filter_map(|item| item.split_once(r#"<div class="ds-thread-sub ds-truncate">"#))
            .filter_map(|(_, rest)| rest.split_once("</div>"))
            .map(|(subject, _)| subject.to_owned())
            .collect()
    }

    /// A label's name finds the conversation that bears it, and the box itself still shows what
    /// was typed, so this is a search and not a filter that silently rewrites the query.
    #[tokio::test(start_paused = true)]
    async fn a_label_name_finds_the_conversation_that_bears_it_and_the_box_keeps_it() {
        let (store, _dir) = labelled();
        let page = typing(store, "label:travel", |page| listed(page) == ["hi"]).await;
        assert_eq!(
            listed(&page),
            vec!["hi"],
            "the window searched for the words instead of the label"
        );
        assert!(
            page.contains(&format!(r#"aria-label="{}""#, crate::ui::command::LABEL))
                && page.contains(r#"value="label:travel""#),
            "the search box lost the text:\n{page}"
        );
    }

    /// The control. Without it the test above would pass on a window that ignores the search
    /// box entirely and shows the Inbox whatever is typed.
    #[tokio::test(start_paused = true)]
    async fn a_label_nothing_bears_finds_nothing() {
        let (store, _dir) = labelled();
        let page = typing(store, "label:nosuchlabel", |page| listed(page).is_empty()).await;
        assert!(
            listed(&page).is_empty(),
            "a search that matches nothing still showed {:?}",
            listed(&page)
        );
    }
}

/// A row is as tall as what it draws: one of two lines is shorter than one of three, and a
/// heading is shorter still. Which a row draws is `row::listed`'s.
#[test]
fn a_row_is_as_tall_as_the_lines_it_draws() {
    use super::pitch;
    use ds::components::app::thread_height::ThreadLines;
    let three = pitch(Some(ThreadLines::Three));
    let two = pitch(Some(ThreadLines::Two));
    let heading = pitch(None);
    assert!(three > two && two > heading, "{three} {two} {heading}");
}

/// What the end of the search shown in the header says: where the search looked, or why it
/// could not be run.
#[test]
fn the_search_shown_says_where_it_looked() {
    use super::{Said, searched};
    use mail_ui_kit::list_search::Scope;
    let broken = regex::Regex::new(&String::from("("))
        .expect_err("an unclosed group")
        .to_string();
    let ada = ["ada@example.test".to_owned()];
    let both = ["ada@example.test".to_owned(), "me@example.test".to_owned()];
    let said = |text: &str, bad: bool| Said {
        text: text.to_owned(),
        bad,
    };
    // A case's name, the list's answer, the accounts looked at, how many there are, and what is said.
    type Case<'a> = (&'a str, Scope, &'a [String], usize, Said);
    let cases: &[Case] = &[
        (
            "the only account",
            Scope::Searched,
            &ada,
            1,
            said("in all mail", false),
        ),
        (
            "one of two, its tile pressed",
            Scope::Searched,
            &ada,
            2,
            said("in ada@example.test", false),
        ),
        (
            "every account",
            Scope::Searched,
            &both,
            2,
            said("in all mail", false),
        ),
        (
            "not answered yet",
            Scope::Place,
            &both,
            2,
            said("in all mail", false),
        ),
        (
            "a bare pattern",
            Scope::OverPage,
            &both,
            2,
            said("on this page of Inbox", false),
        ),
        (
            "a broken pattern",
            Scope::Invalid(broken),
            &both,
            2,
            said("unclosed group", true),
        ),
    ];
    for (name, scope, looked, all, want) in cases {
        assert_eq!(searched(scope, "Inbox", looked, *all), *want, "{name}");
    }
}

/// The quick filter's Unread key is the list's, so a menu that is open takes the key first:
/// with the list's ⋯ menu showing it filters nothing, and with it shut it shows the filter row.
#[tokio::test]
async fn the_unread_filter_key_waits_for_an_open_menu() {
    // The stylesheet in the markup names the class too: the row is the element wearing it.
    const FILTER_ROW: &str = "class=\"quick-filter\"";
    use crate::ui::fixtures::{INSIDE_THE_SHELL, PRIMARY, click, drain_seen, rebuild_into, work};
    use dioxus::html::input_data::keyboard_types::Modifiers;

    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let seen = seen.merge(drain_seen(&mut dom));
    let shell = dioxus_core::ElementId(INSIDE_THE_SHELL as usize);
    let page = until(&mut dom, |page| page.contains("ds-list-item")).await;
    assert!(!page.contains(FILTER_ROW), "the row before any key");

    let more = seen.one("aria-label", mail_ui_model::copy::aria::MORE);
    click(&mut dom, more);
    let _ = drain_seen(&mut dom);
    crate::ui::fixtures::chord(&mut dom, "u", PRIMARY | Modifiers::ALT, shell);
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains(FILTER_ROW),
        "the key filtered the list under an open menu"
    );

    // Escape shuts the menu, and the key is the list's again.
    crate::ui::fixtures::chord(&mut dom, "Escape", Modifiers::empty(), shell);
    let _ = drain_seen(&mut dom);
    crate::ui::fixtures::chord(&mut dom, "u", PRIMARY | Modifiers::ALT, shell);
    let _ = drain_seen(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains(FILTER_ROW), "the key with no menu open");
}
