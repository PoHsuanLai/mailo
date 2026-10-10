//! The Import sheet: a path, what is at it, where it goes, and the run.

use ds::base::press::Press;
use ds::components::content::avatar::AvatarSize;
use ds::components::controls::button_marks::Trailing;
use ds::components::controls::button_model::Answers;
use ds::components::fields::field_row::{FieldGroup, FieldRow, RowLayout};
use ds::components::fields::text_field_model::Invalid;
use ds::host::measure::MountedRef;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use std::sync::Arc;

use super::super::common::in_card;
use dioxus::prelude::*;
use mail_core::SqliteStore;

use super::super::common::classed;
use super::super::debounce::use_debounced;
use super::super::menu::{MenuItem, Right, Tile, anchor_at, narrowed, palette_groups};
use super::super::pick::{Ask, choose};
use super::super::press::{SheetClose, available, on_primary};
use super::work::{self, Dest, Looked};
use super::{Phase, Report, run, tilde_here};
use crate::ui::view::{FileSheet, Shell};

/// The path the sheet's field holds.
fn typed(shell: &Shell) -> String {
    match &shell.files {
        Some(FileSheet::Import { path }) => path.clone(),
        _ => String::new(),
    }
}

fn set_typed(mut shell: Signal<Shell>, path: String) {
    shell.write().files = Some(FileSheet::Import { path });
}

/// The sheet. Mounted while `shell.files` is an import.
#[component]
pub(super) fn ImportSheet(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let path = typed(&shell.read());
    let debounced = use_debounced(shell, typed);
    // The path the sheet opened with is looked at on the first frame; every later one once the
    // field is still, off the thread that draws, since counting reads the whole source.
    let mut looked = use_signal(|| work::look(&work::expand_here(&typed(&shell.peek()))));
    let _look = use_resource(move || {
        let settled = debounced.settled.read().clone();
        async move {
            if settled.generation == 0 {
                return;
            }
            let path = work::expand_here(&settled.text);
            let done = tokio::task::spawn_blocking(move || work::look(&path)).await;
            if let Ok(done) = done
                && debounced.is_latest(settled.generation)
            {
                looked.set(done);
            }
        }
    });
    let mut dest = use_signal(Dest::default);
    let mut menu_open = use_signal(|| false);
    let mut into_button = use_signal(|| None::<MountedRef>);
    let mut find = use_signal(String::new);
    let phase = use_signal(|| Phase::Ready);
    let shown = looked();
    let (validity, look_words) = match &shown {
        Looked::Blank => (
            Validity::Valid,
            "An mbox file, a Maildir or an .eml message".to_owned(),
        ),
        Looked::Mail { said, .. } => (Validity::Valid, said.clone()),
        Looked::Refused(why) => (
            Validity::Invalid(Invalid {
                message: why.clone().into(),
                stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
            }),
            String::new(),
        ),
    };
    let ready = match &shown {
        Looked::Mail { source, count, .. } => Some((source.clone(), *count)),
        _ => None,
    };
    let busy = phase.read().running();
    let can_run = ready.is_some() && !busy;
    let hint = format!("{}/Takeout/All mail.mbox", tilde_here(&super::save_dir()));
    let chosen = dest();
    let note = match chosen {
        Dest::Local => "Stays on this computer",
        Dest::Folder { .. } => "Uploaded to the account",
    };
    // Read only while the menu is open: it lists every IMAP account's folders.
    let items = if menu_open() {
        let store = consume_context::<Arc<SqliteStore>>();
        dest_items(&work::destinations(&store), &chosen)
    } else {
        Vec::new()
    };
    let start = move |_| {
        let Some((source, count)) = ready.clone() else {
            return;
        };
        let into = dest();
        let store = consume_context::<Arc<SqliteStore>>();
        let mut revision = revision;
        run(
            phase,
            count,
            move |report| {
                work::import_then_send(&store, &source, &into, chrono::Utc::now(), &mut |so_far| {
                    report(so_far.read)
                })
            },
            move || revision += 1,
        );
    };
    rsx! {
        Sheet {
            common: in_card(),
            label: "Import mail".to_owned(),
            onclose: move |()| super::close(shell),
            div { class: "sheet-form",
                FieldGroup {
                    FieldRow {
                        label: "From",
                        help: Some(look_words.into()),
                        layout: RowLayout::Form,
                        div { class: "sheet-path",
                            TextField {
                                label: "The file or directory to import".to_owned(),
                                value: path,
                                placeholder: hint,
                                validity,
                                focus: FieldFocus::OnMount,
                                common: classed("sheet-path-field"),
                                oninput: move |value: String| set_typed(shell, value),
                            }
                            Button {
                                label: "File…".to_owned(),
                                title: "Choose an mbox or .eml file".to_owned(),
                                onclick: on_primary(move || {
                                    choose(Ask::File, Some(super::save_dir()), move |paths| {
                                        set_typed(shell, paths[0].display().to_string());
                                    });
                                }),
                            }
                            Button {
                                label: "Folder…".to_owned(),
                                title: "Choose a Maildir directory".to_owned(),
                                onclick: on_primary(move || {
                                    choose(Ask::Folder, Some(super::save_dir()), move |paths| {
                                        set_typed(shell, paths[0].display().to_string());
                                    });
                                }),
                            }
                        }
                    }
                    FieldRow {
                        label: "Into",
                        help: Some(note.into()),
                        layout: RowLayout::Form,
                        Button {
                            label: chosen.label(),
                            icon: if chosen == Dest::Local { Icon::Inbox } else { Icon::Mail },
                            trailing: Some(Trailing::Glyph(Icon::ChevronDown)),
                            shown: Some(if menu_open() { Shown::Visible } else { Shown::Hidden }),
                            onclick: move |_: Press| menu_open.set(!menu_open()),
                            common: Common {
                                aria_label: Some(format!("Import into: {}", chosen.label())),
                                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                                    into_button.set(Some(MountedRef(event.data())));
                                })),
                                ..Common::default()
                            },
                        }
                        if menu_open() {
                            PickList::<String> {
                                anchor: anchor_at(into_button()),
                                label: "Import into",
                                placeholder: "Find a folder",
                                query: find(),
                                groups: palette_groups(&narrowed(&items, &find()), AvatarSize::Size22, None),
                                empty: "No folder matches.",
                                oninput: move |text: String| find.set(text),
                                onpick: move |key: String| {
                                    let store = consume_context::<Arc<SqliteStore>>();
                                    if let Some(found) = work::destinations(&store)
                                        .into_iter()
                                        .find(|one| one.key() == key)
                                    {
                                        dest.set(found);
                                    }
                                },
                                onclose: move |()| {
                                    menu_open.set(false);
                                    find.set(String::new());
                                },
                            }
                        }
                    }
                }
                div { class: "sheet-actions",
                    Report { phase: phase(), verb: "Importing" }
                    SheetClose { label: "Cancel".to_owned(), on_close: move |()| super::close(shell) }
                    Button {
                        answers: Answers::Return,
                        label: if busy { "Importing…" } else { "Import" },
                        availability: available(can_run),
                        onclick: on_primary(move || start(())),
                    }
                }
            }
        }
    }
}

/// The destination menu: local folders, then each IMAP account's folders under its address.
pub(in crate::ui) fn dest_items(all: &[Dest], chosen: &Dest) -> Vec<MenuItem> {
    all.iter()
        .map(|one| {
            let (name, group, icon, help) = match one {
                Dest::Local => (
                    "Local folders".to_owned(),
                    "This computer".to_owned(),
                    Icon::Inbox,
                    Some("Never synced".to_owned()),
                ),
                Dest::Folder {
                    address, folder, ..
                } => (folder.clone(), address.clone(), Icon::Mail, None),
            };
            MenuItem {
                key: one.key(),
                tile: Tile::Icon(icon),
                name,
                help,
                right: Right::Check(one == chosen),
                group: Some(group),
                marks: Vec::new(),
                title: Vec::new(),
                detail: Vec::new(),
            }
        })
        .collect()
}
