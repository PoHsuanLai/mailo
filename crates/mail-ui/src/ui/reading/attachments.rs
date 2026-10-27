//! What a message has attached, and saving it.
//!
//! The parts are a strip of tiles under the body: a thumbnail (`thumb.rs`) that opens Peek, or a
//! paperclip for a part not yet here, over the name and the size, with Save or Download. The tile
//! keeps its width and its one line of name however long the name is: the name is cut short
//! before its extension, which stays.
//!
//! Save writes a part that is already here; Download fetches one still on the server and then
//! writes it. The name is the one the file will be written under. There is no file chooser: it
//! lands in the downloads directory, and a toast says which folder.
//!
//! Each tile's progress is a `mail_client::fetch::Download`: Running turns its button Busy, Saved
//! says so in a toast, Failed says why under the strip until it is dismissed or tried again.
//!
//! A message OpenPGP or S/MIME opened to a body of its own lists what is attached inside it —
//! what is stored is only its wrapping, the signature or the ciphertext. Those parts are in
//! memory, in what the reader was opened to; Save writes one off the thread that draws.

use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::Bezel;
use ds::host::measure::{RectProbe, use_rect};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use std::collections::HashMap;
use std::sync::Arc;

use dioxus::prelude::*;
use mail_client::fetch::{Download, DownloadEffect, DownloadEvent};
use mail_client::message::Looks;
use mail_domain::{BlobId, MessageId, Retry};

use super::super::text::{AttachmentRow, Kept};
use super::fetch::{self, Again};
use super::thumb::Thumb;
use crate::ui::common::classed;
use crate::ui::copy;
use crate::ui::reauth::{Reauth, SIGN_IN};
use crate::ui::view::Shell;

type Downloads = Signal<HashMap<usize, Download>>;

/// The strip for one message. Each tile's download is its own state, kept here by index.
#[component]
pub(super) fn Attachments(
    message: MessageId,
    body: Option<BlobId>,
    rows: Vec<AttachmentRow>,
    shell: Signal<Shell>,
) -> Element {
    let downloads: Downloads = use_signal(HashMap::new);
    let toasts = use_hook(try_consume_context::<ds::stack::toast_hub::ToastHub>);
    let client = crate::ui::live::use_client();
    let looks = super::use_looks();
    let reauth = try_consume_context::<Reauth>();
    let account = use_hook(|| {
        client
            .stored()
            .message(message)
            .map(|m| fetch::account_address(&client, m.account))
            .unwrap_or_default()
    });
    let mut tiles = Vec::new();
    let mut failures = Vec::new();
    for row in &rows {
        let index = row.index;
        let state = downloads
            .read()
            .get(&index)
            .cloned()
            .unwrap_or(Download::Idle);
        let busy = matches!(state, Download::Running { .. });
        let label = match row.kept {
            Kept::Here | Kept::Opened => copy::controls::SAVE,
            Kept::OnServer => copy::controls::DOWNLOAD,
        };
        let name = row.name.clone();
        let kept = row.kept;
        let saved_name = name.clone();
        let again_name = name.clone();
        let sign_in_account = account.clone();
        let saved_looks = looks.clone();
        let again_looks = looks.clone();
        // A stored part leads with its preview (a picture's, a PDF's first page, or the
        // paperclip), which opens Peek; one still on the server, with the paperclip alone.
        let (face, opens) = match kept {
            Kept::Here => (
                rsx! { Thumb { message, index, name: name.clone(), shell } },
                true,
            ),
            Kept::OnServer | Kept::Opened => (
                rsx! { div { class: "att-away", Glyph { icon: Icon::Paperclip, size: IconSize::Compact } } },
                false,
            ),
        };
        tiles.push(rsx! {
            div { key: "{index}", class: "att-tile", role: "listitem",
                {face}
                div {
                    class: "att-text",
                    onclick: move |_| {
                        if opens {
                            super::peek::open(shell, message, index);
                        }
                    },
                    Name { name: name.clone() }
                    Label { text: row.size.clone(), role: LabelRole::Tertiary, style: LabelStyle::Footnote, common: classed("truncate") }
                }
                Button {
                    bezel: Bezel::Inline,
                    label,
                    common: Common { aria_label: Some(format!("{label} {name}")), ..Common::default() },
                    availability: if busy { Availability::Busy } else { Availability::Enabled },
                    onclick: super::super::press::on_primary(move || {
                        start(&saved_looks, message, body, index, kept, &saved_name, downloads, toasts);
                    }),
                }
            }
        });
        if let Download::Failed { retry, why } = &state {
            let failure = fetch::download_failure(&name, retry, why, &account);
            failures.push(rsx! {
                InlineBanner {
                    key: "{index}-failed",
                    severity: Severity::Danger,
                    text: failure.text,
                    detail: Some(failure.detail).filter(|d| !d.is_empty()).map(TextLine::from),
                    actions: match failure.again {
                        Again::Offer => Some(rsx! {
                            Button {
                                label: crate::ui::copy::controls::TRY_AGAIN,
                                common: Common { aria_label: Some(crate::ui::copy::aria::try_again_to_download(&name)), ..Common::default() },
                                onclick: super::super::press::on_primary(move || {
                                    start(&again_looks, message, body, index, kept, &again_name, downloads, toasts);
                                }),
                            }
                        }),
                        Again::SignIn(need) => reauth.map(|reauth| rsx! {
                            Button {
                                label: SIGN_IN,
                                common: Common { aria_label: Some(format!("{SIGN_IN} to {sign_in_account}")), ..Common::default() },
                                onclick: super::super::press::on_primary({
                                    let sign_in_account = sign_in_account.clone();
                                    move || reauth.sign_in(&sign_in_account, need)
                                }),
                            }
                        }),
                        Again::Withhold => None,
                    },
                    onclose: move |()| {
                        send(downloads, index, DownloadEvent::Dismiss);
                    },
                }
            });
        }
    }
    rsx! {
        div { class: "attachments",
            div { class: "att-strip", role: "list", aria_label: "Attachments", {tiles.into_iter()} }
            {failures.into_iter()}
        }
    }
}

/// A tile's name: the stem, cut short where it does not fit, then the extension. The whole name is
/// its tooltip while it is cut, and only then: a name drawn whole says all a tip would. It is cut
/// when the stem and the extension take the row's whole width, which a name that fits leaves room
/// in (it is centred); each is measured as laid out.
#[component]
fn Name(name: String) -> Element {
    let row = use_rect();
    let stem_at = use_rect();
    let extension_at = use_rect();
    let width = |probe: RectProbe| probe.rect().map(|rect| rect.size.width.0);
    let cut = match (width(row), width(stem_at)) {
        (Some(row), Some(stem)) => stem + width(extension_at).unwrap_or(0.0) >= row - 0.5,
        _ => false,
    };
    let (stem, extension) = split_name(&name);
    let stem = rsx! {
        Label {
            text: stem.to_owned(),
            style: LabelStyle::Footnote,
            common: Common {
                mounted: Some(EventHandler::new(move |event| stem_at.on_mounted(event))),
                ..classed("truncate")
            },
        }
    };
    rsx! {
        div { class: "att-name", onmounted: move |event| row.on_mounted(event),
            if cut {
                Tooltip { text: name.clone(), {stem} }
            } else {
                {stem}
            }
            if !extension.is_empty() {
                Label {
                    text: extension.to_owned(),
                    style: LabelStyle::Footnote,
                    common: Common {
                        mounted: Some(EventHandler::new(move |event| extension_at.on_mounted(event))),
                        ..Common::default()
                    },
                }
            }
        }
    }
}

/// A name as its tile shows it: what is cut short when it does not fit, and the extension, kept
/// whole so a long name still says what kind of file it is (`report-…final.pdf`). A name with no
/// extension, or a dot file, is all stem.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && (2..=6).contains(&(name.len() - dot)) => name.split_at(dot),
        _ => (name, ""),
    }
}

/// Move one row's download by `event`; what the machine asks for is returned.
fn send(mut downloads: Downloads, index: usize, event: DownloadEvent) -> Option<DownloadEffect> {
    let from = downloads
        .peek()
        .get(&index)
        .cloned()
        .unwrap_or(Download::Idle);
    let (to, effect) = from.step(event);
    downloads.write().insert(index, to);
    effect
}

type Toasts = Option<ds::stack::toast_hub::ToastHub>;

/// Begin saving one row: from a press, which is where a task is polled.
#[allow(clippy::too_many_arguments)]
fn start(
    looks: &Arc<Looks>,
    message: MessageId,
    body: Option<BlobId>,
    index: usize,
    kept: Kept,
    name: &str,
    downloads: Downloads,
    toasts: Toasts,
) {
    if send(downloads, index, DownloadEvent::Start) != Some(DownloadEffect::Begin) {
        return;
    }
    // What is on the server takes a while, and shows in Downloads while it does.
    let saving = match kept {
        Kept::OnServer => crate::ui::downloads::Saving::begin(name),
        Kept::Here | Kept::Opened => crate::ui::downloads::Saving::quick(),
    };
    let client = crate::ui::live::client();
    let looks = looks.clone();
    let fetchers = fetch::fetchers();
    let dir = crate::ui::files::save_dir();
    spawn(async move {
        // `spawn_blocking`, not this task: a fetch waits on the application's runtime
        // (`Client::net`), and `Runtime::block_on` inside an async context panics.
        let done = tokio::task::spawn_blocking(move || {
            let saved = match kept {
                Kept::Here => client
                    .attach()
                    .save(message, index, &dir)
                    .map_err(String::from),
                Kept::Opened => {
                    super::super::pgp::save_attachment(&looks, message, body, index, &dir)
                }
                Kept::OnServer => fetch::fetch_then_save(&client, &fetchers, message, index, &dir),
            };
            (saved, crate::ui::downloads::origin(&client, message))
        })
        .await;
        let (done, origin) = match done {
            Ok((saved, origin)) => (Ok(saved), origin),
            Err(error) => (Err(error), None),
        };
        saving.end(
            done.as_ref().ok().and_then(|saved| saved.as_deref().ok()),
            origin,
        );
        let event = match done {
            Ok(Ok(path)) => DownloadEvent::Saved(path),
            Ok(Err(why)) => DownloadEvent::Failed {
                retry: Retry::Now,
                why,
            },
            Err(error) => DownloadEvent::Failed {
                retry: Retry::Now,
                why: format!("it stopped early: {error}"),
            },
        };
        let saved = match &event {
            DownloadEvent::Saved(path) => Some(fetch::saved_toast(path)),
            _ => None,
        };
        send(downloads, index, event);
        if let Some(text) = saved {
            // The notice is the outcome; the row has nothing more to say.
            send(downloads, index, DownloadEvent::Dismiss);
            if let Some(toasts) = toasts {
                toasts.push(text, None);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::split_name;

    /// A name keeps its extension apart, and one with no extension is all stem.
    #[test]
    fn a_name_is_split_at_its_extension() {
        const CASES: &[(&str, (&str, &str))] = &[
            ("minutes.pdf", ("minutes", ".pdf")),
            ("photos.tar.gz", ("photos.tar", ".gz")),
            ("Report v2.docx", ("Report v2", ".docx")),
            ("README", ("README", "")),
            (".profile", (".profile", "")),
            ("ends with a dot.", ("ends with a dot.", "")),
            (
                "v1.0 notes for the meeting",
                ("v1.0 notes for the meeting", ""),
            ),
        ];
        for (name, parts) in CASES {
            assert_eq!(split_name(name), *parts, "{name:?}");
        }
    }
}
