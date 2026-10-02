//! The Export sheet: which messages, how many, in which format, to where, and the run.

use ds::components::controls::button_model::Answers;
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::{FieldGroup, FieldRow, RowLayout};
use ds::components::fields::text_field_model::Invalid;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use std::sync::Arc;

use super::super::common::in_card;
use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::common::classed;
use super::super::debounce::use_debounced;
use super::super::pick::{Ask, choose};
use super::super::press::{SheetClose, available, on_primary};
use super::work::{self, Counted, Format};
use super::{Phase, Report, run};
use crate::ui::view::{FileSheet, Shell};

/// The query the sheet's field holds.
fn typed(shell: &Shell) -> String {
    match &shell.files {
        Some(FileSheet::Export { query }) => query.clone(),
        _ => String::new(),
    }
}

/// Where the export goes: the suggestion, until a path is typed or chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Suggested,
    Typed(String),
}

/// The sheet. Mounted while `shell.files` is an export.
#[component]
pub(super) fn ExportSheet(shell: Signal<Shell>) -> Element {
    let query = typed(&shell.read());
    let debounced = use_debounced(shell, typed);
    // `None` while the count for the settled text is being worked out.
    let mut counted = use_signal(|| None::<Counted>);
    let _count = use_resource(move || {
        let settled = debounced.settled.read().clone();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            counted.set(None);
            let text = settled.text.clone();
            let done = tokio::task::spawn_blocking(move || {
                work::counted(&store, &text, chrono::Utc::now())
            })
            .await;
            if let Ok(done) = done
                && debounced.is_latest(settled.generation)
            {
                counted.set(Some(done));
            }
        }
    });
    let mut format = use_signal(Format::default);
    let mut place = use_signal(|| Place::Suggested);
    let phase = use_signal(|| Phase::Ready);
    let settled = debounced.settled.read().text.clone();
    let target_path = match place() {
        Place::Typed(path) => path,
        Place::Suggested => work::suggested(&super::save_dir(), &settled, format())
            .display()
            .to_string(),
    };
    let (validity, count_words, count) = match counted() {
        None => (Validity::Valid, "Counting…".to_owned(), None),
        Some(Counted::Blank) => (Validity::Valid, "A place or a search".to_owned(), None),
        Some(Counted::Some(0)) => (refusal("Nothing matches."), String::new(), None),
        Some(Counted::Some(n)) => (Validity::Valid, work::messages(n), Some(n)),
        Some(Counted::Refused(why)) => (refusal(&why), String::new(), None),
    };
    let busy = phase.read().running();
    let can_run = count.is_some() && !busy && !target_path.trim().is_empty();
    let formats: Vec<Choice<Format>> = Format::ALL
        .iter()
        .map(|one| Choice::new(*one, one.label()))
        .collect();
    let what = if format().is_file() {
        "One mbox file"
    } else if format() == Format::Maildir {
        "A Maildir folder per place"
    } else {
        "One .eml file per message"
    };
    let start = {
        let target_path = target_path.clone();
        move |_| {
            let query = typed(&shell.peek());
            let target = format().target(work::expand_here(&target_path));
            let of = count.unwrap_or(0);
            let store = consume_context::<Arc<SqliteStore>>();
            run(
                phase,
                of,
                move |report| {
                    work::export_now(&store, &query, &target, chrono::Utc::now(), &mut |done| {
                        report(done.written)
                    })
                    .map(|(_, said)| said)
                },
                // The name just used is taken now; the next suggestion is a fresh one.
                move || place.set(Place::Suggested),
            );
        }
    };
    rsx! {
        Sheet {
            common: in_card(),
            label: "Export mail".to_owned(),
            onclose: move |()| super::close(shell),
            div { class: "sheet-form",
                FieldGroup {
                    FieldRow {
                        label: "Which",
                        help: Some(count_words.into()),
                        layout: RowLayout::Form,
                        TextField {
                            label: "Which messages".to_owned(),
                            value: query,
                            placeholder: "inbox, all, from:dana".to_owned(),
                            validity,
                            focus: FieldFocus::OnMount,
                            oninput: move |value: String| {
                                shell.write().files = Some(FileSheet::Export { query: value });
                            },
                        }
                    }
                    FieldRow {
                        label: "As",
                        help: Some(what.into()),
                        layout: RowLayout::Form,
                        SegmentedControl::<Format> {
                            label: "Format".to_owned(),
                            choices: formats,
                            tracking: Tracking::SelectOne(format()),
                            onchange: move |one: Format| format.set(one),
                        }
                    }
                    FieldRow {
                        label: "To",
                        layout: RowLayout::Form,
                        div { class: "sheet-path",
                            TextField {
                                label: "Where to write it".to_owned(),
                                value: target_path.clone(),
                                placeholder: "Where to write it".to_owned(),
                                common: classed("sheet-path-field"),
                                oninput: move |value: String| place.set(Place::Typed(value)),
                            }
                            Button {
                                label: "Folder…".to_owned(),
                                title: "Choose the directory it goes in".to_owned(),
                                onclick: on_primary(move || {
                                    choose(Ask::Folder, Some(super::save_dir()), move |mut dirs| {
                                        let dir = dirs.swap_remove(0);
                                        let settled = debounced.settled.peek().text.clone();
                                        let path = work::suggested(&dir, &settled, format());
                                        place.set(Place::Typed(path.display().to_string()));
                                    });
                                }),
                            }
                        }
                    }
                }
                div { class: "sheet-actions",
                    Report { phase: phase(), verb: "Exporting" }
                    SheetClose { label: "Cancel".to_owned(), on_close: move |()| super::close(shell) }
                    Button {
                        answers: Answers::Return,
                        label: if busy { "Exporting…" } else { "Export" },
                        availability: available(can_run),
                        onclick: on_primary(move || start(())),
                    }
                }
            }
        }
    }
}

/// A refusal quire's field draws under itself: the words, and one stamp for each phrasing.
fn refusal(why: &str) -> Validity {
    Validity::Invalid(Invalid {
        message: why.to_owned().into(),
        stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
    })
}
