//! The saved-view sheet: make a view, change one, or forget it.
//!
//! Opened by "Save as view" beside a search, by "Edit view" while a view is shown, and by
//! "New view…" in the search bar. Drawn in the Rules sheet's shape and with its parts: a view
//! is a search that stays, and the editor that keeps a rule's search is the one people already
//! know. What it writes is [`crate::ui::saved`]'s to decide; this only draws the draft and hands it
//! to the store.

use super::common::Seg;
use super::menu::{Floating, MenuItem, Right, Tile};
use super::press::{SheetClose, on_primary};
use crate::ui::saved::{self, HOVER_CHOICES, ViewDraft, group_choices, group_name, hover_name};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::Answers;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::{OpKind, SortDir, View};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// Open the sheet on a new view of what `search` finds.
pub(in crate::ui) fn open_new(mut shell: Signal<Shell>, search: &str) {
    shell.write().view_editor = Some(ViewDraft::from_search(search));
}

/// Open the sheet on `view`.
pub(in crate::ui) fn open_edit(mut shell: Signal<Shell>, view: &View) {
    let labels = shell.peek().labels.clone();
    shell.write().view_editor = Some(ViewDraft::of(view, &labels, &chrono::Local));
}

/// Close the sheet and give the keyboard back to the window. Nothing typed is kept.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().view_editor = None;
    crate::ui::host::Host::focus_app();
}

/// Change the open draft, if the sheet is still open.
fn edit(mut shell: Signal<Shell>, change: impl FnOnce(&mut ViewDraft)) {
    if let Some(draft) = shell.write().view_editor.as_mut() {
        change(draft);
    }
}

/// Keep the draft as a view and show it, or say why not.
fn save(mut shell: Signal<Shell>, mut revision: Signal<u64>, mut pages: Signal<u32>) {
    let Some(draft) = shell.peek().view_editor.clone() else {
        return;
    };
    let labels = shell.peek().labels.clone();
    let store = consume_context::<Arc<SqliteStore>>();
    let kept = draft.build(&labels, &chrono::Local).and_then(|view| {
        store
            .put_view(&view)
            .map(|()| view)
            .map_err(|e| e.to_string())
    });
    match kept {
        Ok(view) => {
            saved::show_kept(&mut shell.write(), &view);
            pages.set(1);
            revision += 1;
            crate::ui::host::Host::focus_app();
        }
        Err(why) => edit(shell, |draft| draft.refused = Some(why)),
    }
}

/// Forget the view the draft edits.
fn delete(mut shell: Signal<Shell>, mut revision: Signal<u64>, mut pages: Signal<u32>) {
    let Some(id) = shell.peek().view_editor.as_ref().and_then(|draft| draft.id) else {
        return;
    };
    let store = consume_context::<Arc<SqliteStore>>();
    match store.delete_view(id) {
        Ok(()) => {
            saved::show_forgotten(&mut shell.write(), id);
            pages.set(1);
            revision += 1;
            crate::ui::host::Host::focus_app();
        }
        Err(why) => edit(shell, |draft| draft.refused = Some(why.to_string())),
    }
}

fn group_items(draft: &ViewDraft, labels: &[(String, mail_domain::LabelId)]) -> Vec<MenuItem> {
    group_choices(labels)
        .into_iter()
        .map(|(key, name, group)| MenuItem {
            tile: if group.is_none() {
                Tile::Glyph('–')
            } else {
                Tile::Icon(Icon::Group)
            },
            right: Right::Check(draft.group == group),
            key,
            name,
            help: None,
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect()
}

/// The sheet. Mounted while `shell.view_editor` is `Some`.
#[component]
pub(in crate::ui) fn ViewSheet(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    pages: Signal<u32>,
) -> Element {
    let mut grouping = use_signal(|| false);
    let mut group_at = use_signal(|| None::<MountedRef>);
    let Some(draft) = shell.read().view_editor.clone() else {
        return rsx! {};
    };
    let labels = shell.read().labels.clone();
    let editing = draft.id.is_some();
    let title = if editing { "Edit view" } else { "New view" };
    let group_label = group_name(draft.group.as_ref(), &labels);
    let items = group_items(&draft, &labels);
    let oldest = draft.dir == SortDir::Asc;
    rsx! {
        div {
            class: "rules-wrap",
            onclick: move |_| close(shell),
            div {
                class: "rules",
                role: "dialog",
                aria_label: "Saved view",
                onclick: move |event| event.stop_propagation(),
                div { class: "rules-head",
                    h3 { "{title}" }
                    SheetClose { on_close: move |()| close(shell) }
                }
                div { class: "rules-main",
                    div { class: "rules-part",
                        span { class: "files-k", "Name" }
                        TextField {
                            label: "Name".to_owned(),
                            value: draft.name.clone(),
                            placeholder: "Receipts".to_owned(),
                            common: Common {
                                extra_class: ExtraClass::parse("rules-in").ok(),
                                ..Common::default()
                            },
                            oninput: move |value: String| edit(shell, |draft| draft.name = value),
                        }
                        span { class: "files-k", "Lists" }
                        TextField {
                            label: "Lists".to_owned(),
                            value: draft.query.clone(),
                            placeholder: "from:shop.example has:attachment".to_owned(),
                            common: Common {
                                extra_class: ExtraClass::parse("rules-in").ok(),
                                ..Common::default()
                            },
                            oninput: move |value: String| edit(shell, |draft| draft.query = value),
                        }
                        if draft.kept.is_some() && draft.query.trim().is_empty() {
                            p { class: "rules-faint",
                                "Lists what it was saved with, which a search cannot say. Type one to change it."
                            }
                        }
                        span { class: "files-k", "Group by" }
                        div {
                            Button {
                                label: group_label.clone(),
                                icon: Icon::Group,
                                shown: Some(if grouping() { Shown::Visible } else { Shown::Hidden }),
                                common: Common {
                                    aria_label: Some(format!("Group by: {group_label}")),
                                    mounted: Some(EventHandler::new(move |event: MountedEvent| {
                                        group_at.set(Some(MountedRef(event.data())));
                                    })),
                                    ..Common::default()
                                },
                                onclick: on_primary(move || grouping.set(!grouping())),
                            }
                            if grouping() {
                                Floating {
                                    anchor: group_at(),
                                    title: "Group by".to_owned(),
                                    items,
                                    on_pick: move |key: String| {
                                        let labels = shell.peek().labels.clone();
                                        if let Some((_, _, group)) = group_choices(&labels)
                                            .into_iter()
                                            .find(|(word, _, _)| *word == key)
                                        {
                                            edit(shell, |draft| draft.group = group);
                                        }
                                        grouping.set(false);
                                    },
                                    on_close: move |_| grouping.set(false),
                                }
                            }
                        }
                        span { class: "files-k", "Order" }
                        Seg {
                            label: "Order".to_owned(),
                            options: vec![("Newest first".to_owned(), !oldest), ("Oldest first".to_owned(), oldest)],
                            on_pick: move |index: usize| {
                                edit(shell, |draft| draft.dir = if index == 1 { SortDir::Asc } else { SortDir::Desc });
                            },
                        }
                    }
                    div { class: "rules-part",
                        h4 { "Row actions" }
                        p { class: "rules-faint",
                            if draft.hover.is_empty() {
                                "None chosen: each row's menu offers the usual actions."
                            } else {
                                "Each row's menu offers these, where the conversation can take them."
                            }
                        }
                        ul { class: "rules-actions",
                            for kind in HOVER_CHOICES {
                                HoverChoice { key: "{hover_name(kind)}", kind, on: draft.hover.contains(&kind), shell }
                            }
                        }
                    }
                }
                div { class: "rules-part",
                    div { class: "rules-acts",
                        if let Some(why) = draft.refused.clone() {
                            p { class: "capnote files-bad", role: "alert", "{why}" }
                        }
                        if editing {
                            Button {
                                size: ControlSize::Large,
                                label: "Delete view".to_owned(),
                                icon: Icon::Trash,
                                common: Common {
                                    aria_label: Some("Delete view".to_owned()),
                                    ..Common::default()
                                },
                                onclick: on_primary(move || delete(shell, revision, pages)),
                            }
                        }
                        Button {
                            size: ControlSize::Large,
                            label: "Cancel".to_owned(),
                            onclick: on_primary(move || close(shell)),
                        }
                        Button {
                            size: ControlSize::Large,
                            answers: Answers::Return,
                            label: "Save".to_owned(),
                            common: Common {
                                aria_label: Some(format!("Save {}", title.to_lowercase())),
                                ..Common::default()
                            },
                            onclick: on_primary(move || save(shell, revision, pages)),
                        }
                    }
                }
            }
        }
    }
}

/// One hover button the view can offer: a button that stays pressed while it is in.
#[component]
fn HoverChoice(kind: OpKind, on: bool, shell: Signal<Shell>) -> Element {
    let name = hover_name(kind);
    rsx! {
        li {
            Button {
                label: name,
                value: Some(if on { Check::On } else { Check::Off }),
                common: Common {
                    aria_label: Some(format!("Offer {name} on hover")),
                    ..Common::default()
                },
                onclick: on_primary(move || edit(shell, |draft| draft.toggle_hover(kind))),
            }
        }
    }
}
