//! The Contacts sheet: the whole book, a filter, a name to edit and an entry to forget on each
//! row, and vCard in and out.
//!
//! Import takes files through the native dialog Attach uses (`ui::pick`); export writes
//! `contacts.vcf` into the downloads directory the way Save writes an attachment, never over a
//! file already there, and says where it went.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::command::avatar_color;
use super::super::field::{Field, FieldKind};
use super::super::pick::{Ask, choose, file_name};
use super::super::press::{SheetClose, on_primary};
use super::book::{self, Row, SYNC_COMMAND};
use crate::view::Shell;
use ds::{Glyph, Icon};

/// Rows drawn at once. A book of thousands is filtered, not scrolled through.
const SHOWN: usize = 200;

/// The name being edited: whose, and what is typed so far.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Naming {
    address: String,
    typed: String,
}

/// The sheet. Mounted while `shell.contacts` is `Some`, which holds the filter.
#[component]
pub(in crate::ui) fn ContactsSheet(shell: Signal<Shell>) -> Element {
    let filter = shell.read().contacts.clone().unwrap_or_default();
    // Bumped by every write, so the rows are read again.
    let changed = use_signal(|| 0u64);
    let naming = use_signal(|| None::<Naming>);
    let mut said = use_signal(|| None::<String>);
    let listed = use_memo(move || {
        let _ = changed();
        let filter = shell.read().contacts.clone().unwrap_or_default();
        let store = consume_context::<Arc<SqliteStore>>();
        book::rows(store.as_ref(), &filter)
    });
    let (rows, failed) = match listed() {
        Ok(rows) => (rows, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let total = rows.len();
    let count = if total == 1 {
        "1 contact".to_owned()
    } else {
        format!("{total} contacts")
    };
    let empty = if filter.trim().is_empty() {
        "The book is empty. Mail you send and receive fills it."
    } else {
        "Nobody in the book matches."
    };
    rsx! {
        div {
            class: "book-wrap",
            onclick: move |_| super::close(shell),
            div {
                class: "book",
                role: "dialog",
                aria_label: "Contacts",
                onclick: move |event| event.stop_propagation(),
                div { class: "book-head",
                    h3 { "Contacts" }
                    span { class: "count", "{count}" }
                    SheetClose { on_close: move |()| super::close(shell) }
                }
                label { class: "book-find",
                    Glyph { icon: Icon::Search }
                    Field {
                        kind: FieldKind::Inline,
                        value: filter.clone(),
                        placeholder: "Filter by name or address".to_owned(),
                        extra: None,
                        on_input: move |value: String| shell.write().contacts = Some(value),
                        on_focus: |_| {},
                        on_blur: |_| {},
                    }
                }
                ul { class: "book-rows",
                    for row in rows.into_iter().take(SHOWN) {
                        BookRow { key: "{row.address}", row, naming, changed, said }
                    }
                    if total == 0 {
                        li { class: "book-none", "{failed.clone().unwrap_or_else(|| empty.to_owned())}" }
                    }
                    if total > SHOWN {
                        li { class: "book-none", "{SHOWN} of {total} shown. Filter to find the rest." }
                    }
                }
                div { class: "book-foot",
                    div { class: "book-io",
                        ds::Button {
                            variant: ds::ButtonVariant::Mini,
                            label: "Import vCard…".to_owned(),
                            aria_label: "Import vCard…".to_owned(),
                            icon: Icon::Plus,
                            onclick: on_primary(move || {
                                choose(Ask::Cards, None, move |paths| import(paths, changed, said));
                            }),
                        }
                        ds::Button {
                            variant: ds::ButtonVariant::Mini,
                            label: "Export vCard…".to_owned(),
                            icon: Icon::Forward,
                            onclick: on_primary(move || {
                                let store = consume_context::<Arc<SqliteStore>>();
                                let dir = crate::attach::downloads_dir();
                                said.set(Some(match book::export(store.as_ref(), &dir) {
                                    Ok(path) => format!("Saved to {}", path.display()),
                                    Err(why) => why,
                                }));
                            }),
                        }
                    }
                    if let Some(said) = said() {
                        p { class: "capnote said", "{said}" }
                    }
                    p { class: "capnote",
                        "CardDAV address books sync from the command line, not from here:"
                    }
                    code { class: "command", "{SYNC_COMMAND}" }
                }
            }
        }
    }
}

/// Import each of the vCard files at `paths`, in order, each read on a blocking thread, and say
/// what the last one did.
fn import(
    paths: Vec<std::path::PathBuf>,
    mut changed: Signal<u64>,
    mut said: Signal<Option<String>>,
) {
    spawn(async move {
        for path in paths {
            let name = file_name(&path);
            let read = tokio::task::spawn_blocking(move || std::fs::read(path)).await;
            let answer = match read {
                Ok(Ok(bytes)) => {
                    let store = consume_context::<Arc<SqliteStore>>();
                    book::import(store.as_ref(), &bytes)
                }
                _ => Err(format!("Cannot read {name}.")),
            };
            said.set(Some(answer.unwrap_or_else(|why| why)));
            changed += 1;
        }
    });
}

/// One entry: who, where it came from, and its two actions.
#[component]
fn BookRow(
    row: Row,
    naming: Signal<Option<Naming>>,
    changed: Signal<u64>,
    said: Signal<Option<String>>,
) -> Element {
    let shown = row.name.clone().unwrap_or_else(|| row.address.clone());
    let letter = shown
        .chars()
        .next()
        .map(|ch| ch.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let color = avatar_color(&row.address);
    let editing = naming
        .read()
        .as_ref()
        .filter(|open| open.address == row.address)
        .map(|open| open.typed.clone());
    let address = row.address.clone();
    let origin = match row.quiet {
        Some(quiet) => format!("{} · {quiet}", row.standing.label()),
        None => row.standing.label().to_owned(),
    };
    let action = row.standing.name_action();
    rsx! {
        li { class: "book-row",
            span { class: "av", style: "background:{color}", "{letter}" }
            div { class: "who",
                if let Some(typed) = editing {
                    NameField { address: address.clone(), typed, naming, changed, said }
                } else {
                    b { "{shown}" }
                    span { class: "addr", "{row.address}" }
                }
                span { class: "origin", "{origin}" }
            }
            div { class: "acts",
                ds::Button {
                    variant: ds::ButtonVariant::Secondary,
                    label: action.to_owned(),
                    aria_label: format!("{action}: {address}"),
                    onclick: {
                        let address = address.clone();
                        let typed = row.name.clone().unwrap_or_default();
                        on_primary(move || naming.set(Some(Naming { address: address.clone(), typed: typed.clone() })))
                    },
                }
                ds::Button {
                    variant: ds::ButtonVariant::Danger,
                    label: "Forget".to_owned(),
                    aria_label: format!("Forget {address}"),
                    title: "Mail may teach it again".to_owned(),
                    onclick: on_primary({
                        let address = address.clone();
                        move || {
                        let store = consume_context::<Arc<SqliteStore>>();
                        said.set(Some(match book::forget(store.as_ref(), &address) {
                            Ok(_) => format!("Forgot {address}. Mail may teach it again."),
                            Err(why) => why,
                        }));
                        changed += 1;
                        }
                    }),
                }
            }
        }
    }
}

/// The inline name field: Enter keeps the name, Esc puts the row back.
#[component]
fn NameField(
    address: String,
    typed: String,
    naming: Signal<Option<Naming>>,
    changed: Signal<u64>,
    said: Signal<Option<String>>,
) -> Element {
    let keep = {
        let address = address.clone();
        move || {
            let typed = naming
                .peek()
                .as_ref()
                .map(|open| open.typed.clone())
                .unwrap_or_default();
            let store = consume_context::<Arc<SqliteStore>>();
            if let Err(why) = book::name(store.as_ref(), &address, &typed) {
                said.set(Some(why));
            }
            naming.set(None);
            changed += 1;
        }
    };
    let mut keep_on_enter = keep.clone();
    let keep_on_click = keep;
    rsx! {
        div {
            class: "naming",
            onkeydown: move |event: KeyboardEvent| match event.key().to_string().as_str() {
                "Enter" => {
                    event.prevent_default();
                    keep_on_enter();
                }
                "Escape" => {
                    event.stop_propagation();
                    naming.set(None);
                }
                _ => {}
            },
            Field {
                kind: FieldKind::Boxed,
                value: typed,
                placeholder: "Their name".to_owned(),
                extra: Some("book-name".to_owned()),
                on_input: move |value: String| {
                    if let Some(open) = naming.write().as_mut() {
                        open.typed = value;
                    }
                },
                on_focus: |_| {},
                on_blur: |_| {},
            }
            ds::Button {
                variant: ds::ButtonVariant::Primary,
                label: "Save".to_owned(),
                aria_label: format!("Save the name for {address}"),
                onclick: on_primary(keep_on_click),
            }
        }
    }
}
