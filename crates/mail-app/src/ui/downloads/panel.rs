//! The Downloads button at the foot of the sidebar, and the list it opens above itself.
//!
//! A row is only the file: its name, and its size, when it was saved and the message it came
//! from. A click opens it in the app the system gives its kind; everything else is in its right
//! click: Show in Folder, Go to Message, Remove from List and Clear List. Removing and clearing
//! leave every file where it is. A file that has since been moved or deleted stays listed, and
//! says so.

use super::log::Entry;
use super::{Shelf, forget, opener, reread};
use crate::ui::motion::{motion, tell_through};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::label::LabelRole;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::menus::item::item::MenuItem;
use ds::components::overlays::popover::Arrow;
use ds::host::measure::{Anchor, MountedRef};
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use mail_domain::ThreadId;
use std::path::{Path, PathBuf};

/// A row of the list: a file being saved, or one saved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Going(u64),
    Saved(PathBuf),
}

/// What a row's right click offers.
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    Open,
    Reveal,
    Message(ThreadId),
    Remove,
    Clear,
}

/// The right click's menu: where it opened, and on which file.
#[derive(Debug, Clone, PartialEq)]
struct Held {
    at: Point,
    entry: Entry,
}

/// The button, with a dot while a file is being saved or one was saved since the list was last
/// looked at; the list while it is open, and a row's menu while that is.
#[component]
pub(in crate::ui) fn DownloadsButton(shell: Signal<Shell>) -> Element {
    let Some(mut shelf) = try_use_context::<Signal<Shelf>>() else {
        return rsx! {};
    };
    let mut open = use_signal(|| false);
    // The button's hold: what the list stands above, edge to edge with the button.
    let mut hold = use_signal(|| None::<MountedRef>);
    let menu = use_signal(|| None::<Held>);
    let going = !shelf.read().going.is_empty();
    let dot = if going {
        Some("downloads-dot going")
    } else if shelf.read().unseen {
        Some("downloads-dot")
    } else {
        None
    };
    rsx! {
        span { class: "downloads-hold",
            onmounted: move |event: MountedEvent| hold.set(Some(MountedRef(event.data()))),
            // The foot's right click is the Space's menu; here it is nobody's.
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
            },
            Button {
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                icon: Icon::Download,
                label: "Downloads",
                title: "Files you saved".to_owned(),
                onclick: move |_| {
                    if *open.peek() {
                        open.set(false);
                        return;
                    }
                    reread(shelf);
                    if shelf.peek().unseen {
                        shelf.write().unseen = false;
                    }
                    open.set(true);
                },
            }
            if let Some(class) = dot {
                span { class, aria_hidden: "true" }
            }
            if open() {
                DownloadsList {
                    anchor: crate::ui::menu::anchor_at(hold()),
                    shelf,
                    menu,
                    onclose: move |()| open.set(false),
                }
            }
            if let Some(held) = menu() {
                RowMenu { held, shelf, shell, open, menu }
            }
        }
    }
}

#[component]
fn DownloadsList(
    anchor: Anchor,
    shelf: Signal<Shelf>,
    mut menu: Signal<Option<Held>>,
    onclose: EventHandler<()>,
) -> Element {
    let held = shelf.read();
    let empty = held.log.entries.is_empty() && held.going.is_empty();
    let now = chrono::Utc::now();
    let mut items: Vec<ListItem<Key>> = held
        .going
        .iter()
        .map(|going| {
            ListItem::row(
                Key::Going(going.id),
                going.name.clone(),
                rsx! {
                    Row {
                        leading: RowLeading::Icon(icon_of(&going.name)),
                        title: going.name.clone(),
                        detail: Some(TextLine::from("Saving\u{2026}")),
                    }
                },
            )
        })
        .collect();
    items.extend(held.log.entries.iter().map(|entry| {
        let name = entry.name();
        let path = entry.path.clone();
        let here = path.exists();
        let kept = entry.clone();
        ListItem::row(
            Key::Saved(path.clone()),
            name.clone(),
            rsx! {
                div { class: "downloads-row",
                    oncontextmenu: move |event: MouseEvent| {
                        event.prevent_default();
                        event.stop_propagation();
                        let at = event.client_coordinates();
                        menu.set(Some(Held {
                            at: Point { x: Px(at.x as f32), y: Px(at.y as f32) },
                            entry: kept.clone(),
                        }));
                    },
                    Row {
                        leading: RowLeading::Icon(icon_of(&name)),
                        title: name.clone(),
                        detail: Some(TextLine::from(detail(entry, here, now))),
                        common: Common {
                            extra_class: if here { None } else { ExtraClass::parse("gone").ok() },
                            ..Common::default()
                        },
                        // A right click is the menu's, not an open.
                        onclick: crate::ui::press::on_primary(move || open_file(path.clone(), onclose)),
                    }
                }
            },
        )
    }));
    drop(held);
    rsx! {
        Popover {
            anchor,
            placement: Placement::new(Side::Top, Align::Start),
            gap: Px(6.0),
            arrow: Arrow::None,
            common: Common { aria_label: Some("Downloads".to_owned()), ..Common::default() },
            onclose: move |()| {
                // A pick in a row's menu lands outside the list; the menu says what closes.
                if menu.peek().is_none() {
                    onclose.call(());
                }
            },
            div { class: "downloads",
                div { class: "downloads-head",
                    Label { text: "Downloads".to_owned(), role: LabelRole::Secondary }
                }
                if empty {
                    p { class: "downloads-empty", "Attachments and files you save show up here." }
                } else {
                    List::<Key> {
                        label: "Downloads",
                        items,
                        onpick: move |key: Key| {
                            if let Key::Saved(path) = key {
                                open_file(path, onclose);
                            }
                        },
                    }
                }
            }
        }
    }
}

/// A row's right click. Every pick but Remove from List closes the list; that one leaves it open
/// on what is left.
#[component]
fn RowMenu(
    held: Held,
    shelf: Signal<Shelf>,
    mut shell: Signal<Shell>,
    mut open: Signal<bool>,
    mut menu: Signal<Option<Held>>,
) -> Element {
    let mut items = Vec::new();
    if held.entry.path.exists() {
        items.push(MenuItem::new(Pick::Open, "Open"));
        items.push(MenuItem::new(Pick::Reveal, "Show in Folder"));
    }
    if let Some(origin) = &held.entry.origin {
        items.push(MenuItem::new(Pick::Message(origin.thread), "Go to Message"));
    }
    if !items.is_empty() {
        items.push(MenuItem::Separator);
    }
    items.push(MenuItem::new(Pick::Remove, "Remove from List"));
    items.push(MenuItem::new(Pick::Clear, "Clear List"));
    let path = held.entry.path.clone();
    rsx! {
        Menu::<Pick> {
            placement: MenuPlacement::Context,
            anchor: Anchor::Point(held.at),
            items,
            common: Common { aria_label: Some(held.entry.name()), ..Common::default() },
            onpick: move |pick: Pick| {
                menu.set(None);
                match pick {
                    Pick::Open => open_file(path.clone(), EventHandler::new(move |()| open.set(false))),
                    Pick::Reveal => {
                        reveal_file(path.clone());
                        open.set(false);
                    }
                    Pick::Message(thread) => {
                        shell.write().open(thread);
                        open.set(false);
                    }
                    Pick::Remove => forget(shelf, Some(&path)),
                    Pick::Clear => {
                        forget(shelf, None);
                        open.set(false);
                    }
                }
            },
            onclose: move |()| menu.set(None),
        }
    }
}

/// The row's second line: its size, when it was saved and the message it came from; or that the
/// file is no longer where it was saved.
pub(super) fn detail(entry: &Entry, here: bool, now: chrono::DateTime<chrono::Utc>) -> String {
    if !here {
        return "Moved or deleted".to_owned();
    }
    let mut parts = vec![
        mail_core::attach::human_size(entry.bytes),
        crate::ui::menus::when_words(entry.at, now, &chrono::Local),
    ];
    if let Some(origin) = &entry.origin
        && !origin.subject.trim().is_empty()
    {
        parts.push(origin.subject.trim().to_owned());
    }
    parts.join(" \u{b7} ")
}

/// A picture's icon for a picture, a page's for anything else.
pub(super) fn icon_of(name: &str) -> Icon {
    let extension = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "bmp" | "svg" | "tif" | "tiff") => {
            Icon::Image
        }
        _ => Icon::File,
    }
}

fn open_file(path: PathBuf, onclose: EventHandler<()>) {
    let open = opener().open;
    let said = motion();
    onclose.call(());
    // Forever, not this list's: the list closes as the file opens.
    dioxus::core::spawn_forever(async move {
        let done = tokio::task::spawn_blocking(move || open(&path)).await;
        if let Some(why) = failed(done) {
            tell_through(said, why);
        }
    });
}

fn reveal_file(path: PathBuf) {
    let reveal = opener().reveal;
    let said = motion();
    dioxus::core::spawn_forever(async move {
        let done = tokio::task::spawn_blocking(move || reveal(&path)).await;
        if let Some(why) = failed(done) {
            tell_through(said, why);
        }
    });
}

fn failed(done: Result<Result<(), String>, tokio::task::JoinError>) -> Option<String> {
    match done {
        Ok(Ok(())) => None,
        Ok(Err(why)) => Some(why),
        Err(error) => Some(format!("Couldn\u{2019}t open it: {error}")),
    }
}
