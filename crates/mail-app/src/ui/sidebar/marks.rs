//! The small mark at the end of an account's heading or a folder's row: a spinner while mail is
//! being fetched for it, a warning button, with the reason as its hover text, when it could not
//! be. The button opens the Connection Doctor.
//!
//! One component per mark, each reading the fetching state itself, so a pass moving along
//! draws that mark and not the whole sidebar.

use crate::ui::fetching::{Fetching, Mark, account_mark_local, folder_mark};
use crate::ui::press::on_primary;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::press::Propagation;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use porter_core::AccountId;

/// An account's mark.
#[component]
pub(super) fn LinkMark(shell: Signal<Shell>, account: AccountId) -> Element {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return rsx! {};
    };
    let mark = fetching
        .link(account)
        .map_or(Mark::Quiet, |link| account_mark_local(&link));
    drawn(shell, mark)
}

/// A folder's mark.
#[component]
pub(super) fn FolderMark(shell: Signal<Shell>, account: AccountId, path: String) -> Element {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return rsx! {};
    };
    drawn(shell, folder_mark(&fetching.folder(account, &path)))
}

/// A warning is a button, as Mail's is: pressing it opens the Connection Doctor, where the
/// person fixes what the glyph is about. Its words are its hover text and its name.
fn drawn(shell: Signal<Shell>, mark: Mark) -> Element {
    match mark {
        Mark::Quiet => rsx! {},
        Mark::Busy => rsx! { Working {} },
        Mark::Warn(why) => glyph(shell, why, Icon::TriangleAlert),
        Mark::Offline(why) => glyph(shell, why, Icon::WifiOff),
    }
}

fn glyph(shell: Signal<Shell>, why: String, icon: Icon) -> Element {
    rsx! {
        span { class: "fetch-mark",
            Tooltip { text: why.clone(),
                Button {
                    label: why,
                    icon: IconSource::Glyph(icon),
                    image: ImagePosition::Only,
                    bezel: Bezel::Toolbar,
                    size: ControlSize::Mini,
                    // Inside a row: pressing the mark is not pressing the row.
                    propagation: Propagation::Stop,
                    onclick: on_primary(move || crate::ui::doctor::open(shell)),
                }
            }
        }
    }
}

/// The small spinner, turning from the moment it is drawn.
#[component]
fn Working() -> Element {
    let operation = use_hook(|| Operation::Running(PendingToken::start()));
    rsx! {
        span { class: "fetch-mark", "aria-busy": "true",
            ProgressIndicator {
                style: ProgressStyle::Spinner,
                progress: Progress::Unknown(operation),
                size: ControlSize::Mini,
            }
        }
    }
}
