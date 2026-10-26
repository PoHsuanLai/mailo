//! The saved-view sheet: make a view, change one, or forget it.
//!
//! Opened by "Save as view" beside a search, by "Edit view" while a view is shown, and by
//! "New view…" in the search bar. Drawn as the other sheets are, in quire's `Sheet` with a form's
//! rows: a view is a search that stays, and the field that keeps a rule's search is the one
//! people already know. What it writes is [`crate::ui::saved`]'s to decide; this only draws the draft and hands it
//! to the client.

use super::common::{Seg, in_card};
use super::menu::{Floating, MenuItem, Right, Tile};
use super::press::{SheetClose, on_primary};
use crate::ui::copy;
use crate::ui::saved::{self, ViewDraft, group_choices, group_name};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::controls::button_model::Answers;
use ds::components::fields::field_row::{FieldGroup, FieldRow, RowLayout};
use ds::components::fields::text_field_model::Invalid;
use ds::components::overlays::sheet_attach::Attach;
use ds::host::measure::MountedRef;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::{SortDir, View};

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
fn save(mut shell: Signal<Shell>, mut pages: Signal<u32>) {
    let Some(draft) = shell.peek().view_editor.clone() else {
        return;
    };
    let labels = shell.peek().labels.clone();
    let client = crate::ui::live::client();
    let kept = draft.build(&labels, &chrono::Local).and_then(|view| {
        client
            .stored()
            .put_view(&view)
            .map(|()| view)
            .map_err(|why| crate::ui::remedy::plainly(copy::errors::NOT_KEPT_HERE, why))
    });
    match kept {
        Ok(view) => {
            saved::show_kept(&mut shell.write(), &view);
            pages.set(1);
            crate::ui::host::Host::focus_app();
        }
        Err(why) => edit(shell, |draft| draft.refused = Some(why)),
    }
}

/// Forget the view the draft edits.
fn delete(mut shell: Signal<Shell>, mut pages: Signal<u32>) {
    let Some(id) = shell.peek().view_editor.as_ref().and_then(|draft| draft.id) else {
        return;
    };
    let client = crate::ui::live::client();
    match client.stored().delete_view(id) {
        Ok(()) => {
            saved::show_forgotten(&mut shell.write(), id);
            pages.set(1);
            crate::ui::host::Host::focus_app();
        }
        Err(why) => {
            let said = crate::ui::remedy::plainly(copy::errors::NOT_KEPT_HERE, why);
            edit(shell, |draft| draft.refused = Some(said));
        }
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
            ..MenuItem::plain(key, name)
        })
        .collect()
}

/// The sheet. Mounted while `shell.view_editor` is `Some`.
#[component]
pub(in crate::ui) fn ViewSheet(shell: Signal<Shell>, pages: Signal<u32>) -> Element {
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
    let name_refused = refusal(
        draft
            .refused
            .as_deref()
            .filter(|_| draft.name.trim().is_empty()),
    );
    let query_refused = refusal(
        draft
            .refused
            .as_deref()
            .filter(|_| !draft.name.trim().is_empty()),
    );
    let query_help = (draft.kept.is_some() && draft.query.trim().is_empty())
        .then(|| TextLine::from("Lists what it was saved with. Type a search to replace that."));
    rsx! {
        Sheet {
            label: "Saved view",
            attach: Attach::Window,
            common: in_card(),
            onclose: move |()| close(shell),
            div { class: "sheet-form",
                Label { text: title, style: LabelStyle::Title }
                FieldGroup {
                    FieldRow { label: copy::fields::NAME, layout: RowLayout::Form,
                        TextField {
                            label: copy::fields::NAME.to_owned(),
                            value: draft.name.clone(),
                            placeholder: "Receipts".to_owned(),
                            validity: name_refused,
                            oninput: move |value: String| edit(shell, |draft| draft.name = value),
                        }
                    }
                    FieldRow { label: "Lists", help: query_help, layout: RowLayout::Form,
                        TextField {
                            label: "Lists".to_owned(),
                            value: draft.query.clone(),
                            placeholder: "from:shop.example has:attachment".to_owned(),
                            validity: query_refused,
                            oninput: move |value: String| edit(shell, |draft| draft.query = value),
                        }
                    }
                    FieldRow { label: "Group by", layout: RowLayout::Form,
                        div {
                            Button {
                                label: group_label.clone(),
                                icon: Icon::Group,
                                shown: Some(if grouping() { Shown::Visible } else { Shown::Hidden }),
                                common: Common {
                                    aria_label: Some(format!("Group by: {group_label}")),
                                    data: crate::ui::sidebar::tagged("view", "group").data,
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
                    }
                    FieldRow { label: "Order", layout: RowLayout::Form,
                        Seg {
                            label: "Order".to_owned(),
                            options: vec![("Newest first".to_owned(), !oldest), ("Oldest first".to_owned(), oldest)],
                            on_pick: move |index: usize| {
                                edit(shell, |draft| draft.dir = if index == 1 { SortDir::Asc } else { SortDir::Desc });
                            },
                        }
                    }
                }
                div { class: "sheet-actions",
                    if editing {
                        Button {
                            label: copy::controls::DELETE_VIEW.to_owned(),
                            icon: Icon::Trash,
                            common: Common {
                                aria_label: Some(copy::aria::DELETE_VIEW.to_owned()),
                                data: crate::ui::sidebar::tagged("view", "delete").data,
                                ..Common::default()
                            },
                            onclick: on_primary(move || delete(shell, pages)),
                        }
                    }
                    SheetClose { label: copy::controls::CANCEL, on_close: move |()| close(shell) }
                    Button {
                        answers: Answers::Return,
                        label: copy::controls::SAVE.to_owned(),
                        common: Common {
                            aria_label: Some(format!("Save {}", title.to_lowercase())),
                            data: crate::ui::sidebar::tagged("view", "save").data,
                            ..Common::default()
                        },
                        onclick: on_primary(move || save(shell, pages)),
                    }
                }
            }
        }
    }
}

/// A refusal quire's field draws under itself: the words, and one stamp for each phrasing.
fn refusal(why: Option<&str>) -> Validity {
    match why {
        None => Validity::Valid,
        Some(why) => Validity::Invalid(Invalid {
            message: why.to_owned().into(),
            stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
        }),
    }
}
