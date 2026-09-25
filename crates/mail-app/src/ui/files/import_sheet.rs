//! The Import sheet: a path, what is at it, where it goes, and the run.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::debounce::use_debounced;
use super::super::field::{Field, FieldKind};
use super::super::menu::{Floating, MenuItem, Right, Tile};
use super::super::pick::{Ask, choose};
use super::super::press::{SheetClose, available, on_primary};
use super::work::{self, Dest, Looked};
use super::{Phase, Progress, run, tilde_here};
use crate::view::{FileSheet, Shell};
use ds::{Filter, Icon, MenuKind, MountedRef};

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
    let mut into = use_signal(|| None::<MountedRef>);
    let phase = use_signal(|| Phase::Ready);
    let shown = looked();
    let (look_class, look_words) = match &shown {
        Looked::Blank => (
            "files-look",
            "An mbox file, a Maildir directory or one .eml message. ~ is your home.".to_owned(),
        ),
        Looked::Mail { said, .. } => ("files-look found", said.clone()),
        Looked::Refused(why) => ("files-look refused", why.clone()),
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
        Dest::Local => "Kept on this computer: searchable, never synced, never sent anywhere.",
        Dest::Folder { .. } => {
            "Uploaded through the account's outbox; whatever cannot go now waits for the next sync."
        }
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
        div {
            class: "files-wrap",
            onclick: move |_| super::close(shell),
            div {
                class: "files",
                role: "dialog",
                aria_label: "Import mail",
                onclick: move |event| event.stop_propagation(),
                div { class: "files-head",
                    h3 { "Import mail" }
                    SheetClose { on_close: move |()| super::close(shell) }
                }
                div { class: "files-main",
                    span { class: "files-k", "From" }
                    div { class: "files-path",
                        Field {
                            kind: FieldKind::Boxed,
                            value: path,
                            placeholder: hint,
                            extra: Some("files-in".to_owned()),
                            on_input: move |value: String| set_typed(shell, value),
                            on_focus: |_| {},
                            on_blur: |_| {},
                        }
                        ds::Button {
                            variant: ds::ButtonVariant::Mini,
                            label: "File…".to_owned(),
                            title: "Choose an mbox or .eml file".to_owned(),
                            onclick: on_primary(move || {
                                choose(Ask::File, Some(super::save_dir()), move |paths| {
                                    set_typed(shell, paths[0].display().to_string());
                                });
                            }),
                        }
                        ds::Button {
                            variant: ds::ButtonVariant::Mini,
                            label: "Folder…".to_owned(),
                            title: "Choose a Maildir directory".to_owned(),
                            onclick: on_primary(move || {
                                choose(Ask::Folder, Some(super::save_dir()), move |paths| {
                                    set_typed(shell, paths[0].display().to_string());
                                });
                            }),
                        }
                    }
                    p { class: "{look_class}", aria_live: "polite", "{look_words}" }
                    span { class: "files-k", "Into" }
                    div { class: "files-into",
                        ds::Button {
                            variant: ds::ButtonVariant::Quiet,
                            label: chosen.label(),
                            icon: if chosen == Dest::Local { Icon::Inbox } else { Icon::Mail },
                            aria_label: format!("Import into: {}", chosen.label()),
                            trailing: Some(ds::Trailing::Caret),
                            expanded: if menu_open() { ds::Expanded::Open } else { ds::Expanded::Closed },
                            mounted: move |event: MountedEvent| into.set(Some(MountedRef(event.data()))),
                            onclick: move |_: ds::Press| menu_open.set(!menu_open()),
                        }
                        if menu_open() {
                            Floating {
                                kind: MenuKind::Dropdown,
                                anchor: into(),
                                title: "Import into".to_owned(),
                                items,
                                filter: Filter::Typing,
                                on_pick: move |key: String| {
                                    let store = consume_context::<Arc<SqliteStore>>();
                                    if let Some(found) = work::destinations(&store)
                                        .into_iter()
                                        .find(|one| one.key() == key)
                                    {
                                        dest.set(found);
                                    }
                                    menu_open.set(false);
                                },
                                on_close: move |_| menu_open.set(false),
                            }
                        }
                    }
                    p { class: "capnote", "{note}" }
                }
                div { class: "files-foot",
                    Progress { phase: phase(), verb: "Importing" }
                    span { class: "go",
                        ds::Button {
                            variant: ds::ButtonVariant::Primary,
                            label: if busy { "Importing…" } else { "Import" },
                            icon: Icon::Plus,
                            availability: available(can_run),
                            onclick: on_primary(move || start(())),
                        }
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
