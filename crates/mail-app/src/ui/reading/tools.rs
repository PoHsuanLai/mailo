//! The reader toolbar's two menus: where the conversation is shown, and its quieter actions.
//!
//! One view button stands for the three peek modes and a window of its own, its icon the mode in
//! use, as Mail's view pop-up does. The ⋯ is the row's menu less what the toolbar beside it
//! already holds, so the two never disagree about what a conversation can do.

use super::super::row::act::{Pressed, press};
use super::super::row::menu::{Muted, Pick, reader_groups, rows};
use crate::ui::menu::{Floating, MenuItem, Right, Tile, anchor_at};
use crate::ui::menus::{LabelMenu, SnoozeMenu};
use crate::ui::view::{Peek, Shell};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::*;

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
pub(super) fn view_items(current: Peek) -> Vec<MenuItem> {
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
    items.push(super::super::window::menu_item());
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
                items: view_items(peek),
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
    let offered = crate::ui::view::hover_in(shell.read().saved_view(), &summary);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_menu_checks_the_mode_in_use_and_ends_with_a_window() {
        for current in MODES {
            let items = view_items(current);
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
