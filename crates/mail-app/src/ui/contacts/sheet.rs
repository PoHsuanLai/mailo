//! The Contacts sheet: the whole book, a filter, a name to edit and an entry to forget on each
//! row, and vCard in and out.
//!
//! Import takes files through the native dialog Attach uses (`ui::pick`); export writes
//! `contacts.vcf` into the downloads directory the way Save writes an attachment, never over a
//! file already there, and says where it went.

use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::ButtonRole;
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::components::overlays::sheet_width::SheetWidth;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::style::tokens::control_size::ControlSize;
use std::sync::Arc;

use super::super::common::in_card;
use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::pick::{Ask, choose, file_name};
use super::super::press::{SheetClose, on_primary};
use super::book::{self, Row, SYNC_COMMAND};
use crate::view::Shell;

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
        "No contacts"
    } else {
        "No matches"
    };
    let items: Vec<ListItem<String>> = rows
        .into_iter()
        .take(SHOWN)
        .map(|row| {
            let label = row.name.clone().unwrap_or_else(|| row.address.clone());
            let key = row.address.clone();
            ListItem::row(key, label, rsx! { BookRow { row, naming, changed, said } })
        })
        .collect();
    let note = failed.unwrap_or_else(|| {
        if total == 0 {
            empty.to_owned()
        } else if total > SHOWN {
            format!("{SHOWN} of {total} shown. Filter to find the rest.")
        } else {
            String::new()
        }
    });
    rsx! {
        Sheet {
            common: in_card(),
            label: "Contacts".to_owned(),
            onclose: move |()| super::close(shell),
            width: SheetWidth::Wide,
            div { class: "book",
                div { class: "book-head",
                    Label { text: count, role: LabelRole::Secondary }
                    TextField {
                        label: "Filter by name or address".to_owned(),
                        kind: FieldKind::Search,
                        placeholder: "Filter by name or address".to_owned(),
                        value: filter.clone(),
                        focus: FieldFocus::OnMount,
                        oninput: move |value: String| shell.write().contacts = Some(value),
                    }
                }
                div { class: "book-rows",
                    List::<String> {
                        label: "Contacts".to_owned(),
                        items,
                        style: ListStyle::Inset,
                    }
                    if !note.is_empty() {
                        Label { text: note, role: LabelRole::Tertiary }
                    }
                }
                div { class: "sheet-actions",
                    if let Some(said) = said() {
                        Label { text: said, role: LabelRole::Secondary }
                    }
                    Button {
                        label: "Import vCard…".to_owned(),
                        common: Common { aria_label: Some("Import vCard…".to_owned()), ..Common::default() },
                        icon: Icon::Plus,
                        onclick: on_primary(move || {
                            choose(Ask::Cards, None, move |paths| import(paths, changed, said));
                        }),
                    }
                    Button {
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
                    SheetClose { label: "Done".to_owned(), on_close: move |()| super::close(shell) }
                }
                Label {
                    text: "CardDAV syncs from the command line:".to_owned(),
                    role: LabelRole::Tertiary,
                    style: LabelStyle::Footnote,
                }
                Label { text: SYNC_COMMAND.to_owned(), role: LabelRole::Tertiary, style: LabelStyle::Footnote }
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
    let face = AvatarFace {
        initial: shown
            .chars()
            .next()
            .and_then(|ch| ch.to_uppercase().next())
            .unwrap_or('?'),
        size: AvatarSize::Size28,
        tone: AvatarTone::Person(person_hue(&address)),
        shape: AvatarShape::Round,
    };
    let acts = rsx! {
        Button {
            label: action.to_owned(),
            size: ControlSize::Small,
            onclick: {
                let address = address.clone();
                let typed = row.name.clone().unwrap_or_default();
                on_primary(move || naming.set(Some(Naming { address: address.clone(), typed: typed.clone() })))
            },
            common: Common { aria_label: Some(format!("{action}: {address}")), ..Common::default() },
        }
        Button {
            role: ButtonRole::Destructive,
            size: ControlSize::Small,
            label: "Forget".to_owned(),
            title: "Mail may teach it again".to_owned(),
            onclick: on_primary({
                let address = address.clone();
                move || {
                    let store = consume_context::<Arc<SqliteStore>>();
                    said.set(Some(match book::forget(store.as_ref(), &address) {
                        Ok(_) => format!("Forgot {address}"),
                        Err(why) => why,
                    }));
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
            leading: RowLeading::Avatar(face),
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
                size: ControlSize::Small,
                onclick: on_primary(keep_on_click),
                common: Common { aria_label: Some(format!("Save the name for {address}")), ..Common::default() },
            }
        }
    }
}
