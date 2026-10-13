//! The reader toolbar's two menus: where the conversation is shown, and its quieter actions.
//!
//! One view button stands for the three peek modes and a window of its own, its icon the mode in
//! use, as Mail's view pop-up does. The ⋯ is the row's menu less what the toolbar beside it
//! already holds, so the two never disagree about what a conversation can do.
//!
//! The ⋯ ends with Download: each of the conversation's messages as the server sent it, an
//! `.eml` apiece, into the downloads directory, the way an attachment's Save writes there. What
//! is written is the stored bytes, never a rendering, so a message that draws wrongly can be
//! handed on whole.

use super::super::row::act::{Pressed, press};
use super::super::row::menu::{Muted, Pick, reader_groups, rows};
use super::fetch;
use crate::ui::downloads::{Origin, Saving};
use crate::ui::menu::{Floating, MenuItem, Right, Tile, anchor_at};
use crate::ui::menus::{LabelMenu, SnoozeMenu};
use crate::ui::view::{Peek, Shell};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_core::{SqliteStore, Store as _};
use mail_domain::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The view menu's modes, in the order it lists them.
const MODES: [Peek; 3] = [Peek::Side, Peek::CENTER, Peek::FULL];

/// The glyph that stands for `peek`, on the button and beside its row.
fn mode_icon(peek: Peek) -> Icon {
    match peek {
        Peek::Side => Icon::Panel,
        Peek::Float(_) if peek == Peek::FULL => Icon::Maximize,
        Peek::Float(_) => Icon::Square,
    }
}

/// The view menu's rows: each mode, the one in use checked, then a window of its own.
pub(super) fn view_items(current: Peek, keymap: &chordkit::Keymap) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = MODES
        .iter()
        .map(|peek| MenuItem {
            key: peek.slug().to_owned(),
            tile: Tile::Icon(mode_icon(*peek)),
            name: peek.label().to_owned(),
            help: None,
            right: Right::Check(*peek == current),
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect();
    items.push(super::super::window::menu_item(keymap));
    items
}

/// A ref the button's mount fills, for the menu to hang from.
fn mounted_into(mut at: Signal<Option<MountedRef>>) -> Common {
    Common {
        mounted: Some(EventHandler::new(move |event: MountedEvent| {
            at.set(Some(MountedRef(event.data())));
        })),
        ..Common::default()
    }
}

/// The view button: its icon the mode in use, its menu the modes and a window of its own.
#[component]
pub(super) fn ViewMenu(thread: ThreadId, peek: Peek, shell: Signal<Shell>) -> Element {
    let mut open = use_signal(|| false);
    let tool = use_signal(|| None::<MountedRef>);
    let keys = use_keys();
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(mode_icon(peek))),
            label: "View".to_owned(),
            title: Some(format!("View \u{b7} {}", peek.label())),
            common: mounted_into(tool),
            onclick: move |_| open.toggle(),
        }
        if open() {
            Floating {
                anchor: tool(),
                title: String::new(),
                items: keys.with_keymap(|keymap| view_items(peek, keymap)),
                on_pick: move |key: String| {
                    open.set(false);
                    if key == super::super::window::OPEN_KEY {
                        super::super::window::open_in_window(thread);
                    } else if let Some(mode) = MODES.iter().find(|mode| mode.slug() == key) {
                        shell.write().peek = *mode;
                    }
                },
                on_close: move |_| open.set(false),
            }
        }
    }
}

/// Which of the ⋯'s further menus is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Further {
    None,
    Snooze,
    Label,
}

/// The ⋯: the conversation's quieter actions, through the row's own press, so a pick lands on
/// the undo stack and asks first where the row's would.
#[component]
pub(super) fn ReaderMore(
    summary: ThreadSummary,
    shell: Signal<Shell>,
    revision: Signal<u64>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut further = use_signal(|| Further::None);
    let tool = use_signal(|| None::<MountedRef>);
    let id = summary.id;
    let offered = mail_core::view::hover_in(shell.read().saved_view(), &summary);
    let muted = match summary.mute {
        Mute::Muted => Muted::Yes,
        Mute::Unmuted => Muted::No,
    };
    let items = rows(&reader_groups(&offered, summary.read, summary.star), muted);
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(Icon::Ellipsis)),
            label: "More".to_owned(),
            title: Some("More".to_owned()),
            common: mounted_into(tool),
            onclick: move |_| open.toggle(),
        }
        if open() {
            Menu::<Pick> {
                placement: MenuPlacement::Popup,
                anchor: anchor_at(tool()),
                items,
                common: Common {
                    aria_label: Some(format!("Actions for {}", summary.subject)),
                    ..Common::default()
                },
                onpick: move |pick: Pick| {
                    open.set(false);
                    match pick {
                        Pick::Press(Pressed::Op(OpKind::Snooze)) => further.set(Further::Snooze),
                        Pick::Press(Pressed::Op(OpKind::AddLabel)) => further.set(Further::Label),
                        Pick::Press(pressed) => press(shell, revision, id, pressed),
                        Pick::Window => super::super::window::open_in_window(id),
                        Pick::Remind => {}
                        Pick::Download => download(id),
                    }
                },
                onclose: move |_| open.set(false),
            }
        }
        match further() {
            Further::Snooze => rsx! {
                SnoozeMenu {
                    id,
                    shell,
                    revision,
                    anchor: tool(),
                    on_done: move |()| further.set(Further::None),
                }
            },
            Further::Label => rsx! {
                LabelMenu {
                    id,
                    summary: summary.clone(),
                    shell,
                    revision,
                    anchor: tool(),
                    on_done: move |()| further.set(Further::None),
                }
            },
            Further::None => rsx! {},
        }
    }
}

/// What saving a conversation's messages came to.
#[derive(Debug, Default)]
struct Downloaded {
    /// Each file written, and the message it came out of.
    saved: Vec<(PathBuf, Option<Origin>)>,
    /// Messages with only their headers here, so nothing to write.
    absent: usize,
    /// Why it stopped before the last message, if it did.
    failed: Option<String>,
}

/// Write each of `thread`'s messages into `dir`, in the order it holds them. Blocking: off the
/// thread that draws.
fn save_thread(store: &SqliteStore, thread: ThreadId, dir: &Path) -> Downloaded {
    let mut done = Downloaded::default();
    let messages = match store.thread(thread) {
        Ok(loaded) => loaded.messages,
        Err(error) => {
            done.failed = Some(error.to_string());
            return done;
        }
    };
    for message in messages {
        match mail_core::export::save_eml(store, message, dir) {
            Ok(Some(path)) => done
                .saved
                .push((path, crate::ui::downloads::origin(store, message))),
            Ok(None) => done.absent += 1,
            Err(error) => {
                done.failed = Some(String::from(error));
                break;
            }
        }
    }
    done
}

/// The toast after a Download: where the files went, and what was left out.
fn downloaded_toast(saved: &[PathBuf], absent: usize, failed: Option<&str>) -> String {
    if let Some(why) = failed {
        return format!(
            "Couldn't save this conversation. ({})",
            fetch::one_line(why)
        );
    }
    let Some(first) = saved.first() else {
        return match absent {
            1 => "Nothing was saved. This message hasn't been downloaded yet.",
            _ => "Nothing was saved. These messages haven't been downloaded yet.",
        }
        .to_owned();
    };
    let folder = fetch::folder_of(first).unwrap_or("Downloads");
    match (saved.len(), absent) {
        (1, 0) => fetch::saved_toast(first),
        (n, 0) => format!("Saved {n} messages to {folder}"),
        (n, absent) => format!(
            "Saved {n} of {} messages to {folder}. The rest haven't been downloaded yet.",
            n + absent
        ),
    }
}

/// Download: save the conversation's messages, then say where. From a press, which is where a
/// task is polled (F140).
fn download(thread: ThreadId) {
    // The bytes are here already, so nothing shows as being saved: each file joins the list
    // when it is written.
    let saving = Saving::quick();
    let store = consume_context::<Arc<SqliteStore>>();
    let toasts = try_consume_context::<ds::stack::toast_hub::ToastHub>();
    let dir = crate::ui::files::save_dir();
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || save_thread(&store, thread, &dir))
            .await
            .unwrap_or_else(|error| Downloaded {
                failed: Some(format!("it stopped before it finished: {error}")),
                ..Downloaded::default()
            });
        for (path, origin) in &done.saved {
            saving.clone().end(Some(path), origin.clone());
        }
        let paths: Vec<PathBuf> = done.saved.into_iter().map(|(path, _)| path).collect();
        let text = downloaded_toast(&paths, done.absent, done.failed.as_deref());
        if let Some(toasts) = toasts {
            toasts.push(text, None);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_download_toast_says_where_and_what_was_left_out() {
        let saved = |n: usize| -> Vec<PathBuf> {
            (0..n)
                .map(|i| PathBuf::from(format!("/home/someone/Downloads/{i}.eml")))
                .collect()
        };
        let cases: &[(Vec<PathBuf>, usize, Option<&str>, &str)] = &[
            (saved(1), 0, None, "Saved to Downloads"),
            (saved(3), 0, None, "Saved 3 messages to Downloads"),
            (
                saved(2),
                1,
                None,
                "Saved 2 of 3 messages to Downloads. The rest haven't been downloaded yet.",
            ),
            (
                saved(0),
                1,
                None,
                "Nothing was saved. This message hasn't been downloaded yet.",
            ),
            (
                saved(0),
                2,
                None,
                "Nothing was saved. These messages haven't been downloaded yet.",
            ),
            (
                saved(1),
                0,
                Some("disk full\nmore"),
                "Couldn't save this conversation. (disk full)",
            ),
        ];
        for (saved, absent, failed, want) in cases {
            assert_eq!(downloaded_toast(saved, *absent, *failed), *want);
        }
    }

    #[test]
    fn the_view_menu_checks_the_mode_in_use_and_ends_with_a_window() {
        for current in MODES {
            let items = view_items(
                current,
                &chordkit::Keymap::conventional(chordkit::Platform::MacOs),
            );
            let checked: Vec<&str> = items
                .iter()
                .filter(|item| item.right == Right::Check(true))
                .map(|item| item.name.as_str())
                .collect();
            assert_eq!(checked, [current.label()]);
            assert_eq!(
                items.last().map(|item| item.key.as_str()),
                Some(super::super::super::window::OPEN_KEY)
            );
        }
    }
}
