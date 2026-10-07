//! The Contacts page of Settings: the whole book, a filter, a name to edit and an entry to forget
//! on each row, the groups, and vCard in and out.
//!
//! Import takes files through the native dialog Attach uses (`ui::pick`); export writes
//! `contacts.vcf` into the downloads directory the way Save writes an attachment, never over a
//! file already there, and says where it went.

use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::ButtonRole;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use std::sync::Arc;

use super::super::common::{Told, classed, person_tile};
use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::pick::{Ask, choose, file_name};
use super::super::press::on_primary;
use super::book::{self, Row, SYNC_COMMAND};
use super::group_rows::GroupRows;
use crate::ui::view::Shell;

/// Rows drawn at once. A book of thousands is filtered, not scrolled through.
const SHOWN: usize = 200;

/// The keys of the list's filter row and its closing note, which no address can be.
const FIND: &str = " find";
const NOTE: &str = " note";

/// The name being edited: whose, and what is typed so far.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Naming {
    address: String,
    typed: String,
}

/// The page: the people, filtered, then the groups, then vCard in and out and CardDAV.
#[component]
pub(in crate::ui) fn ContactsPage(shell: Signal<Shell>) -> Element {
    let filter = shell.read().contacts.clone();
    // Bumped by every write, so the rows are read again.
    let changed = use_signal(|| 0u64);
    let naming = use_signal(|| None::<Naming>);
    let said = use_signal(|| None::<Result<String, String>>);
    let listed = use_memo(move || {
        let _ = changed();
        let filter = shell.read().contacts.clone();
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
        "No contacts yet"
    } else {
        "No one matches"
    };
    // Why the list is empty is a row in it; that it is cut short is said under the group.
    let note = failed.or_else(|| (total == 0).then(|| empty.to_owned()));
    let cut =
        (total > SHOWN).then(|| format!("{SHOWN} of {total} shown. Filter to find the rest."));
    let mut items: Vec<ListItem<String>> = vec![ListItem::row(
        FIND.to_owned(),
        count.clone(),
        rsx! {
            Row {
                title: count,
                size: RowSize::Settings,
                accessory: Accessory::Slot(rsx! {
                    TextField {
                        label: "Filter by name or address".to_owned(),
                        kind: FieldKind::Search,
                        placeholder: "Filter by name or address".to_owned(),
                        value: filter.clone(),
                        common: classed("book-find"),
                        oninput: move |value: String| shell.write().contacts = value,
                    }
                }),
            }
        },
    )];
    items.extend(rows.into_iter().take(SHOWN).map(|row| {
        let label = row.name.clone().unwrap_or_else(|| row.address.clone());
        let key = row.address.clone();
        ListItem::row(key, label, rsx! { BookRow { row, naming, changed, said } })
    }));
    if let Some(note) = note {
        items.push(ListItem::row(
            NOTE.to_owned(),
            note.clone(),
            rsx! { Row { title: TextLine::Runs(vec![TextRun::new(note, RunTone::Faint)]), size: RowSize::Settings } },
        ));
    }
    rsx! {
        Form {
            Told { said: said() }
            FormSection { title: Some("People".to_owned()), footer: cut,
                List::<String> { label: "Contacts".to_owned(), items, style: ListStyle::Grouped }
            }
            GroupRows { filter: filter.clone(), changed, said }
            ImportExport { changed, said }
        }
    }
}

/// vCard in and out, and how CardDAV syncs.
#[component]
fn ImportExport(changed: Signal<u64>, mut said: Signal<Option<Result<String, String>>>) -> Element {
    rsx! {
        FormSection { title: Some("Import and export".to_owned()),
            FieldRow {
                label: "Import",
                help: Some(TextLine::from("People from vCard files, added to the book.")),
                Button {
                    label: "Import vCard…".to_owned(),
                    common: Common { aria_label: Some("Import vCard…".to_owned()), ..Common::default() },
                    onclick: on_primary(move || {
                        choose(Ask::Cards, None, move |paths| import(paths, changed, said));
                    }),
                }
            }
            FieldRow {
                label: "Export",
                help: Some(TextLine::from(format!("The whole book as {} in your downloads.", book::EXPORT_NAME))),
                Button {
                    label: "Export vCard…".to_owned(),
                    onclick: on_primary(move || {
                        let store = consume_context::<Arc<SqliteStore>>();
                        let dir = mail_core::attach::downloads_dir();
                        said.set(Some(
                            book::export(store.as_ref(), &dir)
                                .map(|path| format!("Saved to {}", path.display())),
                        ));
                    }),
                }
            }
            FieldRow {
                label: "CardDAV",
                help: Some(TextLine::Runs(vec![
                    TextRun::new("Syncs from the command line: ", RunTone::Plain),
                    TextRun::new(SYNC_COMMAND, RunTone::Code),
                ])),
            }
        }
    }
}

/// Import each of the vCard files at `paths`, in order, each read on a blocking thread, and say
/// what the last one did.
fn import(
    paths: Vec<std::path::PathBuf>,
    mut changed: Signal<u64>,
    mut said: Signal<Option<Result<String, String>>>,
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
            said.set(Some(answer));
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
    said: Signal<Option<Result<String, String>>>,
) -> Element {
    let shown = row.name.clone().unwrap_or_else(|| row.address.clone());
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
    let acts = rsx! {
        Button {
            label: action.to_owned(),
            onclick: {
                let address = address.clone();
                let typed = row.name.clone().unwrap_or_default();
                on_primary(move || naming.set(Some(Naming { address: address.clone(), typed: typed.clone() })))
            },
            common: Common { aria_label: Some(format!("{action}: {address}")), ..Common::default() },
        }
        Button {
            role: ButtonRole::Destructive,
            label: "Forget".to_owned(),
            title: "Mail may teach it again".to_owned(),
            onclick: on_primary({
                let address = address.clone();
                move || {
                    let store = consume_context::<Arc<SqliteStore>>();
                    said.set(Some(
                        book::forget(store.as_ref(), &address).map(|_| format!("Forgot {address}")),
                    ));
                    changed += 1;
                }
            }),
            common: Common { aria_label: Some(format!("Forget {address}")), ..Common::default() },
        }
    };
    let content = editing.map(|typed| {
        rsx! { NameField { address: address.clone(), typed, naming, changed, said } }
    });
    rsx! {
        Row {
            leading: person_tile(&shown, &address),
            size: RowSize::Settings,
            title: shown,
            detail: Some(format!("{}  ·  {origin}", row.address).into()),
            content,
            accessory: Accessory::Slot(acts),
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
    said: Signal<Option<Result<String, String>>>,
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
                said.set(Some(Err(why)));
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
            TextField {
                label: "Their name".to_owned(),
                value: typed,
                placeholder: "Their name".to_owned(),
                oninput: move |value: String| {
                    if let Some(open) = naming.write().as_mut() {
                        open.typed = value;
                    }
                },
                common: Common { extra_class: ExtraClass::parse("book-name").ok(), ..Common::default() },
            }
            Button {
                label: "Save".to_owned(),
                onclick: on_primary(keep_on_click),
                common: Common { aria_label: Some(format!("Save the name for {address}")), ..Common::default() },
            }
        }
    }
}
