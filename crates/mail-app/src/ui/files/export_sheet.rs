//! The Export sheet: which messages, how many, in which format, to where, and the run.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::debounce::use_debounced;
use super::super::field::{Field, FieldKind};
use super::super::space_editor::Seg;
use super::pick::{Ask, choose};
use super::work::{self, Counted, Format};
use super::{Phase, Progress, run};
use crate::view::{FileSheet, Shell};
use ds::{Glyph, Icon};

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
    let (count_class, count_words, count) = match counted() {
        None => ("files-look", "Counting…".to_owned(), None),
        Some(Counted::Blank) => (
            "files-look",
            "A search, or a place: inbox, sent, archive, all…".to_owned(),
            None,
        ),
        Some(Counted::Some(0)) => ("files-look refused", "Nothing matches.".to_owned(), None),
        Some(Counted::Some(n)) => ("files-look found", work::messages(n), Some(n)),
        Some(Counted::Refused(why)) => ("files-look refused", why, None),
    };
    let busy = phase.read().running();
    let can_run = count.is_some() && !busy && !target_path.trim().is_empty();
    let options: Vec<(String, bool)> = Format::ALL
        .iter()
        .map(|one| (one.label().to_owned(), *one == format()))
        .collect();
    let what = if format().is_file() {
        "One mbox file. An existing file is never written over."
    } else if format() == Format::Maildir {
        "A Maildir; places and labels become its folders."
    } else {
        "A directory with one .eml file per message."
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
        div {
            class: "files-wrap",
            onclick: move |_| super::close(shell),
            div {
                class: "files",
                role: "dialog",
                aria_label: "Export mail",
                onclick: move |event| event.stop_propagation(),
                div { class: "files-head",
                    h3 { "Export mail" }
                    button {
                        class: "mini",
                        r#type: "button",
                        onclick: move |_| super::close(shell),
                        "Close"
                        span { class: "k", "Esc" }
                    }
                }
                div { class: "files-main",
                    span { class: "files-k", "Which" }
                    div { class: "files-path",
                        Field {
                            kind: FieldKind::Boxed,
                            value: query,
                            placeholder: "inbox, sent, all, or a search: from:dana after:2026-01-01".to_owned(),
                            extra: Some("files-in".to_owned()),
                            on_input: move |value: String| {
                                shell.write().files = Some(FileSheet::Export { query: value });
                            },
                            on_focus: |_| {},
                            on_blur: |_| {},
                        }
                    }
                    p { class: "{count_class}", aria_live: "polite", "{count_words}" }
                    span { class: "files-k", "As" }
                    div { class: "files-format",
                        Seg {
                            label: "Format".to_owned(),
                            options,
                            on_pick: move |index: usize| {
                                if let Some(one) = Format::ALL.get(index) {
                                    format.set(*one);
                                }
                            },
                        }
                        span { class: "capnote", "{what}" }
                    }
                    span { class: "files-k", "To" }
                    div { class: "files-path",
                        Field {
                            kind: FieldKind::Boxed,
                            value: target_path.clone(),
                            placeholder: "Where to write it".to_owned(),
                            extra: Some("files-in files-to".to_owned()),
                            on_input: move |value: String| place.set(Place::Typed(value)),
                            on_focus: |_| {},
                            on_blur: |_| {},
                        }
                        button {
                            class: "mini",
                            r#type: "button",
                            title: "Choose the directory it goes in",
                            onclick: move |_| {
                                choose(Ask::Folder, super::save_dir(), move |dir| {
                                    let settled = debounced.settled.peek().text.clone();
                                    let path = work::suggested(&dir, &settled, format());
                                    place.set(Place::Typed(path.display().to_string()));
                                });
                            },
                            "Folder…"
                        }
                    }
                }
                div { class: "files-foot",
                    Progress { phase: phase(), verb: "Exporting" }
                    button {
                        class: "mini primary",
                        r#type: "button",
                        disabled: !can_run,
                        onclick: start,
                        Glyph { icon: Icon::Forward }
                        if busy { "Exporting…" } else { "Export" }
                    }
                }
            }
        }
    }
}
