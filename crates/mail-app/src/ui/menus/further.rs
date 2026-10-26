//! The further menu a conversation's action opened, drawn by whatever opened it: the row, against
//! the point its menu stood at, or the reader, against its ⋯. Which one is open, and over which
//! conversation, is [`Shell::further`]; every way out of a menu clears it.

use super::{LabelMenu, SnoozeMenu};
use crate::ui::move_to::MoveMenu;
use crate::ui::view::{Further, Opened, Shell};
use dioxus::prelude::*;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use mail_domain::ThreadSummary;

/// The further menu open over `summary`, when `opened` is where it was asked for.
#[component]
pub(in crate::ui) fn FurtherMenus(
    summary: ThreadSummary,
    opened: Opened,
    shell: Signal<Shell>,
    /// The opener's box, which the menu hangs from until `placed` is known.
    anchor: Option<MountedRef>,
    /// Where the menu that asked for it stood, which wins over `anchor`.
    #[props(default)]
    placed: Option<Rect>,
) -> Element {
    let id = summary.id;
    let Some(menu) = shell
        .read()
        .further
        .filter(|menu| menu.thread == id && menu.opened == opened)
    else {
        return rsx! {};
    };
    match menu.further {
        Further::Snooze => rsx! {
            SnoozeMenu { id, shell, anchor, placed }
        },
        Further::Label => rsx! {
            LabelMenu { id, summary, shell, anchor, placed }
        },
        Further::Move => rsx! {
            MoveMenu {
                thread: id,
                shell,
                anchor,
                placed,
                on_close: move |_| shell.write().further = None,
            }
        },
    }
}
