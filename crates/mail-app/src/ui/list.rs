//! The list pane.
//!
//! The search box, what the pane says when it has nothing to show, the rows, and the way to
//! ask for another page. Split from [`super::app`] (`CONVENTIONS.md` §8). The queries stay in
//! `App`; this reads the memos it is handed rather than cloning their answers in the parent.

use super::data::account_rows;
use super::field::{Field, FieldKind};
use super::icon::{Glyph, Icon};
use super::list_search::{Marking, RowHit, Scope, row_hit};
use super::ops::start_new;
use super::page::{PageMenus, group_page};
use super::row::{DraftRow, Row};
use crate::provider::provider;
use crate::view::{Nothing, Shell, SyncState, synced};
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::SqliteStore;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The conversations and drafts for wherever the shell is looking.
#[component]
pub(super) fn ThreadList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    revision: Signal<u64>,
    in_a_field: Signal<bool>,
    threads: Memo<Vec<ThreadSummary>>,
    drafts: Memo<Vec<Draft>>,
    nothing: Memo<Nothing>,
    more: Memo<bool>,
    sync_state: Signal<SyncState>,
    entering: Signal<bool>,
    marking: Memo<Marking>,
) -> Element {
    let rows = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        account_rows(&store)
    });
    let place = shell
        .read()
        .places
        .get(shell.read().selected)
        .map(|place| place.name.clone())
        .unwrap_or_else(|| "Inbox".to_owned());
    let address = shell.read().account.and_then(|id| {
        shell
            .read()
            .accounts
            .iter()
            .find(|(_, account)| *account == id)
            .map(|(address, _)| address.clone())
            .or_else(|| {
                rows()
                    .iter()
                    .find(|row| row.id == id)
                    .map(|row| row.address.clone())
            })
    });
    let names: BTreeMap<LabelId, String> = shell
        .read()
        .labels
        .iter()
        .map(|(name, id)| (*id, name.clone()))
        .collect();
    let inbox = place == "Inbox" && shell.read().search.trim().is_empty();
    let note = sync_state.read().message().map(|text| text.to_owned());
    let search_note = marking.read().note();
    let invalid = matches!(marking.read().scope, Scope::Invalid(_));
    let bad = sync_state.read().is_failure();
    enum Line {
        Head(String),
        Mail {
            index: usize,
            summary: Box<ThreadSummary>,
            via: Option<crate::provider::Provider>,
            chips: Vec<String>,
            hit: Option<RowHit>,
        },
    }
    let mut lines = Vec::new();
    let mut row_index = 0usize;
    for band in group_page(
        threads(),
        shell.read().group,
        &names,
        chrono::Utc::now(),
        &chrono::Local,
    ) {
        if let Some(title) = band.title {
            lines.push(Line::Head(title));
        }
        for summary in band.threads {
            let via = rows()
                .iter()
                .find(|row| row.id == summary.account)
                .map(|row| provider(&row.plan));
            let chips = summary
                .labels
                .iter()
                .filter_map(|id| names.get(id).cloned())
                .collect::<Vec<_>>();
            let hit = row_hit(&summary, &marking.read().highlight);
            lines.push(Line::Mail {
                index: row_index,
                summary: Box::new(summary),
                via,
                chips,
                hit,
            });
            row_index += 1;
        }
    }
    rsx! {
        div { class: "list-col",
            div { class: "list-bar",
                h2 {
                    "{place}"
                    if let Some(address) = address {
                        span { class: "mono", "{address}" }
                    }
                }
                if let Some(note) = note {
                    span { class: if bad { "status bad" } else { "status" }, "{note}" }
                }
                if let Some(said) = search_note {
                    span { class: if invalid { "status bad" } else { "status" }, "{said}" }
                }
                div { class: "bar-tools",
                    PageMenus { shell }
                    button {
                        class: "mini",
                        aria_label: "Sync now",
                        disabled: !sync_state.read().may_start(),
                        onclick: move |_| {
                            if !sync_state.read().may_start() {
                                return;
                            }
                            sync_state.set(SyncState::Running);
                            let store = consume_context::<Arc<SqliteStore>>();
                            spawn(async move {
                                // `spawn_blocking`, not this task: sync::run opens sockets and
                                // builds its own runtime, and `Runtime::block_on` inside an async
                                // context panics.
                                let done = tokio::task::spawn_blocking(move || {
                                    crate::sync::run(store, chrono::Utc::now())
                                })
                                .await;
                                sync_state.set(match done {
                                    Ok(result) => synced(result.map(|ran| ran.text)),
                                    Err(e) => synced(Err(format!("the sync pass stopped: {e}"))),
                                });
                                revision += 1;
                            });
                        },
                        Glyph { icon: Icon::Refresh, class: None }
                    }
                    button {
                        class: "mini",
                        title: "Write a new message (c)",
                        onclick: move |_| {
                            let store = consume_context::<Arc<SqliteStore>>();
                            let known = shell.peek().accounts.clone();
                            match start_new(&store, &known) {
                                Ok(draft) => {
                                    shell.write().compose(&draft);
                                    revision += 1;
                                }
                                Err(why) => eprintln!("compose: {why}"),
                            }
                        },
                        Glyph { icon: Icon::Pen, class: None }
                        "Compose"
                    }
                }
            }
            label { class: "search",
                Glyph { icon: Icon::Search, class: None }
                Field {
                    kind: FieldKind::Boxed,
                    value: shell.read().search.clone(),
                    placeholder: "Search all mail".to_owned(),
                    extra: Some("search".to_owned()),
                    on_input: move |value| {
                        shell.write().search = value;
                        pages.set(1);
                    },
                    on_focus: move |_| in_a_field.set(true),
                    on_blur: move |_| in_a_field.set(false),
                }
            }
            ul {
                class: if entering() { "list entering" } else { "list" },
                onanimationend: move |_| entering.set(false),
                if threads().is_empty() && drafts().is_empty() {
                    li { class: "empty",
                        p { "{nothing().message()}" }
                        if nothing().command().is_none() && matches!(nothing(), Nothing::EmptyFolder) {
                            p { class: "mono", if inbox { "inbox zero" } else { "empty" } }
                        }
                        if let Some(command) = nothing().command() {
                            pre { class: "command", "{command}" }
                        }
                    }
                }
                for (index, draft) in drafts().into_iter().enumerate() {
                    {
                        let id = draft.id;
                        rsx! { DraftRow { key: "{id}", draft, shell, index } }
                    }
                }
                for line in lines {
                    match line {
                        Line::Head(title) => rsx! { li { key: "band-{title}", class: "list-g", "{title}" } },
                        Line::Mail {
                            index,
                            summary,
                            via,
                            chips,
                            hit,
                        } => {
                            let summary = *summary;
                            let id = summary.id;
                            rsx! { Row { key: "{id}", summary, shell, revision, index, chips, via, hit } }
                        }
                    }
                }
            }
            if more() {
                button {
                    class: "mini",
                    onclick: move |_| pages += 1,
                    "Show more"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::App;
    use crate::ui::fixtures::{ACCOUNT, Typed, dispatching, seeded};
    use dioxus::prelude::*;
    use dioxus_core::{NoOpMutations, VirtualDom};
    use mail_domain::*;
    use mail_store::{SqliteStore, Store};
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

    /// Let the off-thread reads finish and fold their answers back into the tree.
    ///
    /// Since phase 8c the list and the badges are computed on a blocking thread and delivered
    /// through a `use_resource`, which keeps its previous value while it recomputes — so one
    /// render after a keystroke shows what was on screen *before* it. That is the right
    /// behaviour in a window, where a blank pane between keystrokes is worse than a stale one,
    /// and the wrong thing to assert against.
    ///
    /// Bounded, and it stops as soon as the tree has nothing left to do: a bare `wait_for_work`
    /// on a settled tree never returns.
    async fn settle(dom: &mut VirtualDom) {
        for _ in 0..16 {
            if tokio::time::timeout(std::time::Duration::from_millis(20), dom.wait_for_work())
                .await
                .is_err()
            {
                break;
            }
            dom.render_immediate(&mut NoOpMutations);
        }
        dom.render_immediate(&mut NoOpMutations);
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

        /// Mount `App`, type `typed` into its search box, and return the rendered page.
        async fn typing(store: Arc<SqliteStore>, typed: &str) -> String {
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
            settle(&mut dom).await;
            dioxus_ssr::render(&dom)
        }

        /// The subjects the list pane is showing.
        ///
        /// Read from the subject cells rather than searched for in the page: the stylesheet is
        /// in the markup, and `page.contains("hi")` is true of `white-space` and `this`. That is
        /// the substring rule in `CONVENTIONS.md`, caught here by a test of its own making.
        fn listed(page: &str) -> Vec<String> {
            page.split(r#"<div class="row-sub">"#)
                .skip(1)
                .filter_map(|rest| rest.split_once("</div>"))
                .map(|(subject, _)| subject.to_owned())
                .collect()
        }

        #[tokio::test]
        async fn a_label_name_finds_the_conversation_that_bears_it() {
            let (store, _dir) = labelled();
            let page = typing(store, "label:travel").await;
            assert_eq!(
                listed(&page),
                vec!["hi"],
                "the window searched for the words instead of the label"
            );
        }

        /// The control. Without it the test above would pass on a window that ignores the search
        /// box entirely and shows the Inbox whatever is typed.
        #[tokio::test]
        async fn a_label_nothing_bears_finds_nothing() {
            let (store, _dir) = labelled();
            let page = typing(store, "label:nosuchlabel").await;
            assert!(
                listed(&page).is_empty(),
                "a search that matches nothing still showed {:?}",
                listed(&page)
            );
        }

        /// And the box itself still shows what was typed, so this is a search and not a filter
        /// that silently rewrites the query.
        #[tokio::test]
        async fn the_box_keeps_what_was_typed() {
            let (store, _dir) = labelled();
            let page = typing(store, "label:travel").await;
            assert!(
                page.contains(r#"class="inp search""#)
                    && page.contains(r#"placeholder="Search all mail""#)
                    && page.contains(r#"value="label:travel""#),
                "the search box lost the text:\n{page}"
            );
        }
    }
}
