//! The list pane, driven through the real `App`.

use super::super::app::App;
use crate::ui::fixtures::{ACCOUNT, Typed, dispatching, seeded};
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use searching_in_the_window::listed;
use std::sync::Arc;

/// Records which `ElementId` each dynamic attribute landed on.
///
/// The one thing a test needs in order to drive the real `App` rather than a stand-in:
/// `handle_event` addresses an element by id, and nothing else in the harness says which id
/// is which. Static attributes live in the template and never appear here, so an element is
/// found by an attribute the component computes — `value` on the search box.
use dioxus_core::ElementId;

#[derive(Default)]
struct WhereThingsWent {
    attrs: Vec<(String, String, dioxus_core::ElementId)>,
}

impl WhereThingsWent {
    /// The element a dynamic `name` attribute was set on, where its value matched.
    fn with_attr(&self, name: &str, matching: impl Fn(&str) -> bool) -> Vec<ElementId> {
        self.attrs
            .iter()
            .filter(|(n, v, _)| n == name && matching(v))
            .map(|(_, _, id)| *id)
            .collect()
    }
}

impl dioxus_core::WriteMutations for WhereThingsWent {
    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &dioxus_core::AttributeValue,
        id: ElementId,
    ) {
        let rendered = match value {
            dioxus_core::AttributeValue::Text(t) => t.clone(),
            other => format!("{other:?}"),
        };
        self.attrs.push((name.to_owned(), rendered, id));
    }

    fn append_children(&mut self, _: ElementId, _: usize) {}
    fn assign_node_id(&mut self, _: &'static [u8], _: ElementId) {}
    fn create_placeholder(&mut self, _: ElementId) {}
    fn create_text_node(&mut self, _: &str, _: ElementId) {}
    fn load_template(&mut self, _: dioxus_core::Template, _: usize, _: ElementId) {}
    fn replace_node_with(&mut self, _: ElementId, _: usize) {}
    fn replace_placeholder_with_nodes(&mut self, _: &'static [u8], _: usize) {}
    fn insert_nodes_after(&mut self, _: ElementId, _: usize) {}
    fn insert_nodes_before(&mut self, _: ElementId, _: usize) {}
    fn set_node_text(&mut self, _: &str, _: ElementId) {}
    fn create_event_listener(&mut self, _: &'static str, _: ElementId) {}
    fn remove_event_listener(&mut self, _: &'static str, _: ElementId) {}
    fn remove_node(&mut self, _: ElementId) {}
    fn push_root(&mut self, _: ElementId) {}
}

/// Render until `landed` holds for the page, doing the tree's pending work in between.
///
/// Since phase 8c the list is computed on a blocking thread, and a search waits for the box to
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
                ACCOUNT,
                Ingest {
                    mailbox: MailboxRef {
                        account: ACCOUNT,
                        path: "INBOX".to_owned(),
                    },
                    validity: UidValidity::Same,
                    cursor: None,
                    messages: vec![],
                    flags: vec![],
                    labels: vec![],
                    label_names: vec![(
                        RemoteRef::Pop {
                            uidl: "u1".to_owned(),
                        },
                        vec!["travel".to_owned()],
                    )],
                    gone: vec![],
                },
            )
            .unwrap();
        (store, dir)
    }

    /// Mount `App`, type `typed` into its search box, let the box go still, and return the
    /// page once `landed` holds for it.
    async fn typing(store: Arc<SqliteStore>, typed: &str, landed: impl Fn(&str) -> bool) -> String {
        dispatching();
        let mut dom = VirtualDom::new(App).with_root_context(store);
        let mut seen = WhereThingsWent::default();
        dom.rebuild(&mut seen);
        // Let the mount-time effects run: the label index is one of them, and the whole
        // question is whether it is there by the time someone types.
        tokio::time::timeout(std::time::Duration::from_millis(500), dom.wait_for_work())
            .await
            .ok();
        dom.render_immediate(&mut NoOpMutations);

        // The search box is the only element whose `value` the component computes; the
        // composer's inputs exist only once a draft is open, and none is.
        let boxes = seen.with_attr("value", |_| true);
        assert_eq!(
            boxes.len(),
            1,
            "expected exactly one dynamic value attribute, found {boxes:?}"
        );
        #[allow(deprecated)]
        dom.handle_event(
            "input",
            std::rc::Rc::new(PlatformEventData::new(Box::new(Typed(typed.to_owned())))),
            boxes[0],
            true,
        );
        tokio::time::advance(crate::ui::debounce::QUIET).await;
        until(&mut dom, landed).await
    }

    /// The subjects the list pane is showing.
    ///
    /// Read from the subject cells rather than searched for in the page: the stylesheet is
    /// in the markup, and `page.contains("hi")` is true of `white-space` and `this`. That is
    /// the substring rule in `CONVENTIONS.md`, caught here by a test of its own making.
    pub(super) fn listed(page: &str) -> Vec<String> {
        page.split(r#"<div class="row-sub">"#)
            .skip(1)
            .filter_map(|rest| rest.split_once("</div>"))
            .map(|(subject, _)| subject.to_owned())
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn a_label_name_finds_the_conversation_that_bears_it() {
        let (store, _dir) = labelled();
        let page = typing(store, "label:travel", |page| listed(page) == ["hi"]).await;
        assert_eq!(
            listed(&page),
            vec!["hi"],
            "the window searched for the words instead of the label"
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

    /// And the box itself still shows what was typed, so this is a search and not a filter
    /// that silently rewrites the query.
    #[tokio::test(start_paused = true)]
    async fn the_box_keeps_what_was_typed() {
        let (store, _dir) = labelled();
        let page = typing(store, "label:travel", |page| listed(page) == ["hi"]).await;
        assert!(
            page.contains(r#"class="inp search""#)
                && page.contains(r#"placeholder="Search all mail""#)
                && page.contains(r#"value="label:travel""#),
            "the search box lost the text:\n{page}"
        );
    }
}
