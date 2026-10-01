//! The attachment viewer: a stored picture, or a PDF a page at a time, larger, over the window.
//!
//! Opened by a thumbnail in the reader's strip (`thumb.rs`) and held in `Shell::viewing`, so the
//! window's keyboard gives it Esc (close) and the arrows (a PDF's pages) before anything else,
//! and opening another conversation or closing the reader closes it. The part is read and drawn
//! on a blocking thread from a resource that runs again only when the part or the page changes.
//! Save writes the same part as the strip's Save.

use std::sync::Arc;

use dioxus::prelude::*;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::MessageId;
use mail_store::{SqliteStore, Store};

use crate::preview::{self, Kind, Page, Picture, Unshown};
use crate::view::{Shell, Viewing};

/// What the viewer draws.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Drawn {
    Picture(Picture),
    Page(Page),
    /// Why nothing is drawn.
    Said(String),
}

/// Attachment `index` of `message`, page `page` if it is a PDF. Blocking: reads and decodes.
fn draw(store: &SqliteStore, message: MessageId, index: usize, page: u32) -> Drawn {
    match preview::load(store, message, index) {
        Ok((Kind::Pdf, bytes)) => match preview::pdf_page(bytes, page, preview::PAGE) {
            Ok(page) => Drawn::Page(page),
            Err(refusal) => Drawn::Said(refusal.sentence()),
        },
        Ok((kind, bytes)) => match preview::picture(&bytes, kind, preview::VIEW) {
            Ok(picture) => Drawn::Picture(picture),
            Err(refusal) => Drawn::Said(refusal.sentence()),
        },
        Err(Unshown::Refused(refusal)) => Drawn::Said(refusal.sentence()),
        Err(Unshown::NotHere) => Drawn::Said("Not downloaded yet".to_owned()),
        Err(Unshown::NotAPicture) => Drawn::Said("No preview for this file".to_owned()),
        Err(Unshown::Store(why)) => Drawn::Said(format!("No preview: {why}")),
    }
}

/// A key while the viewer is open. It takes every key: Esc closes, the arrows and Page Up and
/// Down turn a PDF's pages, and nothing reaches the conversation behind it.
pub(in crate::ui) fn viewer_key(mut shell: Signal<Shell>, key: &str) {
    let by = match key {
        "Escape" => {
            close(shell);
            return;
        }
        "ArrowRight" | "ArrowDown" | "PageDown" => 1,
        "ArrowLeft" | "ArrowUp" | "PageUp" => -1,
        _ => return,
    };
    let turned = shell.peek().viewing.map(|viewing| viewing.turned(by));
    if turned != shell.peek().viewing {
        shell.write().viewing = turned;
    }
}

fn close(mut shell: Signal<Shell>) {
    shell.write().viewing = None;
    crate::ui::host::Host::focus_app();
}

fn turn(mut shell: Signal<Shell>, by: i32) {
    let turned = shell.peek().viewing.map(|viewing| viewing.turned(by));
    shell.write().viewing = turned;
}

/// The viewer. Mounted while `shell.viewing` is `Some`.
#[component]
pub(in crate::ui) fn AttachmentViewer(shell: Signal<Shell>) -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    // The part and the page, and nothing else of the shell: the count written back below must
    // not draw the page again.
    let wanted = use_memo(move || {
        shell
            .read()
            .viewing
            .map(|viewing| (viewing.message, viewing.index, viewing.page))
    });
    let drawn = use_resource(move || {
        let store = consume_context::<Arc<SqliteStore>>();
        let wanted = wanted();
        async move {
            let (message, index, page) = wanted?;
            let drawn = tokio::task::spawn_blocking(move || draw(&store, message, index, page))
                .await
                .unwrap_or_else(|error| Drawn::Said(format!("No preview: {error}")));
            if let Drawn::Page(page) = &drawn {
                let mut shell = shell;
                let known = shell.peek().viewing;
                if let Some(viewing) = known.filter(|v| (v.message, v.index) == (message, index)) {
                    let settled = Viewing {
                        page: page.number,
                        pages: Some(page.count),
                        ..viewing
                    };
                    if settled != viewing {
                        shell.write().viewing = Some(settled);
                    }
                }
            }
            Some(drawn)
        }
    });
    let mut said = use_signal(|| None::<String>);
    let Some(viewing) = shell.read().viewing else {
        return rsx! {};
    };
    let name = store
        .message(viewing.message)
        .ok()
        .and_then(|message| {
            message
                .attachments
                .get(viewing.index)
                .map(|a| crate::attach::safe_name(&a.name))
        })
        .unwrap_or_default();
    let drawn = drawn.read().clone().flatten();
    let pages = matches!(drawn, Some(Drawn::Page(_)))
        .then_some(viewing.pages)
        .flatten();
    let save = move || {
        let store = consume_context::<Arc<SqliteStore>>();
        let dir = super::super::files::save_dir();
        said.set(Some(
            match crate::attach::save(&store, viewing.message, viewing.index, &dir) {
                Ok(path) => format!("Saved to {}", path.display()),
                Err(why) => why,
            },
        ));
    };
    rsx! {
        div { class: "viewer-wrap",
            div { class: "viewer", role: "dialog", "aria-label": "{name}",
                div { class: "viewer-head",
                    h3 { class: "viewer-name", "{name}" }
                    if let Some(count) = pages {
                        span { class: "viewer-page mono", "Page {viewing.page + 1} of {count}" }
                        Button {
                            size: ControlSize::Small,
                            label: "Previous page".to_owned(),
                            availability: super::super::press::available(viewing.page > 0),
                            onclick: super::super::press::on_primary(move || turn(shell, -1)),
                        }
                        Button {
                            size: ControlSize::Small,
                            label: "Next page".to_owned(),
                            availability: super::super::press::available(viewing.page + 1 < count),
                            onclick: super::super::press::on_primary(move || turn(shell, 1)),
                        }
                    }
                    Button {
                        size: ControlSize::Small,
                        label: "Save".to_owned(),
                        onclick: super::super::press::on_primary(save),
                    }
                    Button {
                        size: ControlSize::Small,
                        label: "Close".to_owned(),
                        onclick: super::super::press::on_primary(move || close(shell)),
                    }
                }
                div { class: "viewer-main",
                    match drawn {
                        None => rsx! { p { class: "viewer-note", "Loading…" } },
                        Some(Drawn::Picture(picture)) => rsx! {
                            img { class: "viewer-picture", alt: "{name}", src: "{picture.uri}" }
                        },
                        Some(Drawn::Page(page)) => rsx! {
                            img {
                                class: "viewer-picture viewer-sheet",
                                alt: "{name}, page {page.number + 1}",
                                src: "{page.picture.uri}",
                            }
                        },
                        Some(Drawn::Said(why)) => rsx! { p { class: "viewer-note", "{why}" } },
                    }
                }
                if let Some(said) = said() {
                    p { class: "viewer-said", "{said}" }
                }
            }
        }
    }
}
