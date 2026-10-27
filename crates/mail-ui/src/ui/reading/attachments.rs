//! What a message has attached, and saving it.
//!
//! The parts are a strip of tiles under the body: a thumbnail (`thumb.rs`) that opens Peek, or a
//! paperclip for a part not yet here, over the name and the size, with Save or Download. The tile
//! keeps its width and its one line of name however long the name is: the name is cut short
//! before its extension, which stays.
//!
//! Save writes a part that is already here; Download fetches one still on the server and then
//! writes it. The name is the one the file will be written under. Save asks nothing: it lands in
//! the downloads folder (`attachments.downloads_folder`, else the system's), and a toast says
//! which. A tile's right click offers Save to…, which asks for a folder first; Save All… writes
//! every part into one folder it asks for, in one go, with one toast; the reader's ⋯ has both
//! as Save Attachments… for the message it means ([`files_meant`]). A name already taken in the
//! folder gets " (2)" and on (`mail_core::attach::write_new`): nothing is written over.
//!
//! Each tile's progress is a `mail_client::fetch::Download`: Running turns its button Busy, Saved
//! says so in a toast, Failed says why under the strip until it is dismissed or tried again.
//!
//! A message OpenPGP or S/MIME opened to a body of its own lists what is attached inside it —
//! what is stored is only its wrapping, the signature or the ciphertext. Those parts are in
//! memory, in what the reader was opened to; Save writes one off the thread that draws.

use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::Bezel;
use ds::components::menus::item::item::MenuItem;
use ds::host::measure::{Anchor, RectProbe, use_rect};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use mail_client::Client;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use crate::ui::pick::{Ask, choose};
use crate::ui::reauth::{Reauth, SIGN_IN};
use crate::ui::view::Shell;

type Downloads = Signal<HashMap<usize, Download>>;

/// One part to save: where it is in the message, where its bytes are, and its name.
#[derive(Debug, Clone, PartialEq)]
struct Part {
    index: usize,
    kept: Kept,
    name: String,
}

/// What a tile's right click offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Save,
    SaveTo,
    SaveAll,
}

/// The right click's menu: where it opened, and on which part.
#[derive(Debug, Clone, PartialEq)]
struct Held {
    at: Point,
    part: Part,
}

/// The message the reader's ⋯ › Save Attachments… means among `messages`, each with how many
/// files it shows: the one opened alone (`focus`) when it has files, else the newest that has
/// some. A conversation of messages without files, or a folded message, means none.
pub(super) fn files_meant(
    messages: &[(MessageId, usize)],
    focus: Option<MessageId>,
) -> Option<(MessageId, usize)> {
    match focus.and_then(|focus| messages.iter().find(|(id, _)| *id == focus)) {
        Some(&(id, files)) => (files > 0).then_some((id, files)),
        None => messages.iter().rev().find(|(_, files)| *files > 0).copied(),
    }
}

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
    let mut menu = use_signal(|| None::<Held>);
    let mut picked = use_signal(|| None::<(Pick, Part)>);
    let parts: Vec<Part> = rows
        .iter()
        .map(|row| Part {
            index: row.index,
            kept: row.kept,
            name: row.name.clone(),
        })
        .collect();
    let all = Saver {
        looks: looks.clone(),
        message,
        body,
        downloads,
        toasts,
    };
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
        let held_part = Part {
            index,
            kept,
            name: name.clone(),
        };
        tiles.push(rsx! {
            div { key: "{index}", class: "att-tile", role: "listitem",
                oncontextmenu: move |event: MouseEvent| {
                    event.prevent_default();
                    event.stop_propagation();
                    let at = event.client_coordinates();
                    menu.set(Some(Held {
                        at: Point { x: Px(at.x as f32), y: Px(at.y as f32) },
                        part: held_part.clone(),
                    }));
                },
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
                        let dir = crate::ui::files::save_dir();
                        start(&saved_looks, message, body, index, kept, &saved_name, downloads, toasts, dir);
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
                                    let dir = crate::ui::files::save_dir();
                                    start(&again_looks, message, body, index, kept, &again_name, downloads, toasts, dir);
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
    let several = parts.len() > 1;
    let busy = downloads
        .read()
        .values()
        .any(|state| matches!(state, Download::Running { .. }));
    // A pick from a tile's menu is carried out here, not in the menu: the menu closes on the
    // pick, and the dialog and the save it starts are tasks that must outlive it.
    use_effect({
        let all = all.clone();
        let parts = parts.clone();
        move || {
            let Some((pick, part)) = picked() else {
                return;
            };
            picked.set(None);
            match pick {
                Pick::Save => all.save(part, crate::ui::files::save_dir()),
                Pick::SaveTo => all.save_to(part),
                Pick::SaveAll => all.save_all_to(parts.clone()),
            }
        }
    });
    // The reader's ⋯ › Save Attachments…: this message's files into a folder asked for, as Save All…
    // is, or Save to… when there is one.
    use_effect({
        let all = all.clone();
        let parts = parts.clone();
        move || {
            if shell.read().save != Some(message) {
                return;
            }
            shell.write().save = None;
            match parts.as_slice() {
                [] => {}
                [only] => all.save_to(only.clone()),
                _ => all.save_all_to(parts.clone()),
            }
        }
    });
    rsx! {
        div { class: "attachments",
            if several {
                div { class: "att-head",
                    Button {
                        bezel: Bezel::Inline,
                        label: copy::controls::SAVE_ALL,
                        availability: if busy { Availability::Busy } else { Availability::Enabled },
                        common: Common { id: Some(format!("{SAVE_ALL_ID}-{message}")), ..Common::default() },
                        onclick: super::super::press::on_primary({
                            let parts = parts.clone();
                            move || all.save_all_to(parts.clone())
                        }),
                    }
                }
            }
            div { class: "att-strip", role: "list", aria_label: "Attachments", {tiles.into_iter()} }
            {failures.into_iter()}
            if let Some(held) = menu() {
                TileMenu { held, several, menu, picked }
            }
        }
    }
}

/// The element id Save All… carries, before its message's id.
pub(in crate::ui) const SAVE_ALL_ID: &str = "att-save-all";

/// A tile's right click: Save (or Download), Save to…, and Save All… when there are several.
#[component]
fn TileMenu(
    held: Held,
    several: bool,
    mut menu: Signal<Option<Held>>,
    mut picked: Signal<Option<(Pick, Part)>>,
) -> Element {
    let save = match held.part.kept {
        Kept::Here | Kept::Opened => copy::menus::SAVE,
        Kept::OnServer => copy::menus::DOWNLOAD,
    };
    let mut items = vec![
        MenuItem::new(Pick::Save, save),
        MenuItem::new(Pick::SaveTo, copy::menus::SAVE_TO),
    ];
    if several {
        items.push(MenuItem::Separator);
        items.push(MenuItem::new(Pick::SaveAll, copy::menus::SAVE_ALL));
    }
    let part = held.part.clone();
    rsx! {
        Menu::<Pick> {
            placement: MenuPlacement::Context,
            anchor: Anchor::Point(held.at),
            items,
            common: Common { aria_label: Some(copy::aria::attachment_menu(&held.part.name)), ..Common::default() },
            onpick: move |pick: Pick| {
                picked.set(Some((pick, part.clone())));
                menu.set(None);
            },
            onclose: move |()| menu.set(None),
        }
    }
}

/// What saving any of one message's parts needs, from the strip.
#[derive(Clone)]
struct Saver {
    looks: Arc<Looks>,
    message: MessageId,
    body: Option<BlobId>,
    downloads: Downloads,
    toasts: Toasts,
}

impl Saver {
    /// Save `part` into `dir`, as its tile's button does.
    fn save(&self, part: Part, dir: PathBuf) {
        start(
            &self.looks,
            self.message,
            self.body,
            part.index,
            part.kept,
            &part.name,
            self.downloads,
            self.toasts,
            dir,
        );
    }

    /// Ask for a folder, then save `part` into it.
    fn save_to(&self, part: Part) {
        let saver = self.clone();
        choose(
            Ask::Folder,
            Some(crate::ui::files::save_dir()),
            move |folders| {
                if let Some(dir) = folders.into_iter().next() {
                    saver.save(part, dir);
                }
            },
        );
    }

    /// Ask for a folder, then save every one of `parts` into it.
    fn save_all_to(&self, parts: Vec<Part>) {
        let saver = self.clone();
        choose(
            Ask::Folder,
            Some(crate::ui::files::save_dir()),
            move |folders| {
                if let Some(dir) = folders.into_iter().next() {
                    save_all(&saver, parts, dir);
                }
            },
        );
    }
}

/// Write every one of `parts` into `dir`, one after another off the thread that draws, and say
/// how many in one toast. A part already being saved is passed over; one that fails says why
/// under the strip, as its own Save would.
fn save_all(saver: &Saver, parts: Vec<Part>, dir: PathBuf) {
    let downloads = saver.downloads;
    let begun: Vec<(Part, crate::ui::downloads::Saving)> = parts
        .into_iter()
        .filter(|part| {
            send(downloads, part.index, DownloadEvent::Start) == Some(DownloadEffect::Begin)
        })
        .map(|part| {
            let saving = saving_for(&part);
            (part, saving)
        })
        .collect();
    if begun.is_empty() {
        return;
    }
    let client = crate::ui::live::client();
    let looks = saver.looks.clone();
    let fetchers = fetch::fetchers();
    let (message, body, toasts) = (saver.message, saver.body, saver.toasts);
    let wanted: Vec<Part> = begun.iter().map(|(part, _)| part.clone()).collect();
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            let saved: Vec<Result<PathBuf, String>> = wanted
                .iter()
                .map(|part| write(&client, &looks, &fetchers, message, body, part, &dir))
                .collect();
            (saved, crate::ui::downloads::origin(&client, message))
        })
        .await;
        let (saved, origin) = match done {
            Ok((saved, origin)) => (saved, origin),
            Err(error) => {
                let why = stopped_early(&error);
                (begun.iter().map(|_| Err(why.clone())).collect(), None)
            }
        };
        let mut written = Vec::new();
        for ((part, saving), result) in begun.into_iter().zip(saved) {
            if let Some(path) = finish(downloads, part.index, saving, result, origin.clone()) {
                written.push(path);
            }
        }
        let folder = written
            .first()
            .and_then(|path| fetch::folder_of(path).map(str::to_owned));
        if let (Some(toasts), Some(folder)) = (toasts, folder) {
            toasts.push(copy::toasts::saved_all(written.len(), &folder), None);
        }
    });
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

/// Begin saving one row into `dir`: from a press, which is where a task is polled.
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
    dir: PathBuf,
) {
    if send(downloads, index, DownloadEvent::Start) != Some(DownloadEffect::Begin) {
        return;
    }
    let part = Part {
        index,
        kept,
        name: name.to_owned(),
    };
    let saving = saving_for(&part);
    let client = crate::ui::live::client();
    let looks = looks.clone();
    let fetchers = fetch::fetchers();
    spawn(async move {
        // `spawn_blocking`, not this task: a fetch waits on the application's runtime
        // (`Client::net`), and `Runtime::block_on` inside an async context panics.
        let done = tokio::task::spawn_blocking(move || {
            let saved = write(&client, &looks, &fetchers, message, body, &part, &dir);
            (saved, crate::ui::downloads::origin(&client, message))
        })
        .await;
        let (saved, origin) = match done {
            Ok((saved, origin)) => (saved, origin),
            Err(error) => (Err(stopped_early(&error)), None),
        };
        if let Some(path) = finish(downloads, index, saving, saved, origin)
            && let Some(toasts) = toasts
        {
            toasts.push(fetch::saved_toast(&path), None);
        }
    });
}

/// Why a save's task ended with no answer.
fn stopped_early(error: &tokio::task::JoinError) -> String {
    format!("it stopped early: {error}")
}

/// How a part's save reports to Downloads: what is on the server takes a while, and shows there
/// while it does.
fn saving_for(part: &Part) -> crate::ui::downloads::Saving {
    match part.kept {
        Kept::OnServer => crate::ui::downloads::Saving::begin(part.name.clone()),
        Kept::Here | Kept::Opened => crate::ui::downloads::Saving::quick(),
    }
}

/// Write `part` into `dir`, fetching it first when it is still on the server. Blocking.
fn write(
    client: &Client,
    looks: &Looks,
    fetchers: &fetch::Fetchers,
    message: MessageId,
    body: Option<BlobId>,
    part: &Part,
    dir: &Path,
) -> Result<PathBuf, String> {
    match part.kept {
        Kept::Here => client
            .attach()
            .save(message, part.index, dir)
            .map_err(String::from),
        Kept::Opened => super::super::pgp::save_attachment(looks, message, body, part.index, dir),
        Kept::OnServer => fetch::fetch_then_save(client, fetchers, message, part.index, dir),
    }
}

/// A row's save is over: Downloads is told, and the row moves to Saved (said in a toast by the
/// caller, so its notice is dismissed here) or Failed. Returns the file written.
fn finish(
    downloads: Downloads,
    index: usize,
    saving: crate::ui::downloads::Saving,
    saved: Result<PathBuf, String>,
    origin: Option<crate::ui::downloads::Origin>,
) -> Option<PathBuf> {
    // Downloads lists only a file written; why one was not is the row's to say.
    saving.end(saved.as_deref().ok(), origin);
    match saved {
        Ok(path) => {
            send(downloads, index, DownloadEvent::Saved(path.clone()));
            // The notice is the outcome; the row has nothing more to say.
            send(downloads, index, DownloadEvent::Dismiss);
            Some(path)
        }
        Err(why) => {
            send(
                downloads,
                index,
                DownloadEvent::Failed {
                    retry: Retry::Now,
                    why,
                },
            );
            None
        }
    }
}

#[cfg(test)]
mod meant_tests {
    use super::files_meant;
    use mail_domain::MessageId;

    #[test]
    fn the_menu_means_the_message_opened_alone_else_the_newest_with_files() {
        let [a, b, c] = [(); 3].map(|()| MessageId::generate());
        assert_eq!(files_meant(&[(a, 2), (b, 1), (c, 0)], None), Some((b, 1)));
        assert_eq!(
            files_meant(&[(a, 2), (b, 1), (c, 0)], Some(a)),
            Some((a, 2))
        );
        assert_eq!(files_meant(&[(a, 2), (b, 1)], Some(c)), Some((b, 1)));
        assert_eq!(
            files_meant(&[(a, 2), (b, 0)], Some(b)),
            None,
            "opened alone, no files"
        );
        assert_eq!(files_meant(&[(a, 0), (b, 0)], None), None);
        assert_eq!(files_meant(&[], None), None);
    }
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
