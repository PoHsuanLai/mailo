//! The small mark at the end of an account's heading or a folder's row: a spinner while mail is
//! being fetched for it, a quiet warning, with the reason as its hover text, when it could not be.
//!
//! One component per mark, each reading the fetching state itself, so a pass moving along
//! draws that mark and not the whole sidebar.

use crate::ui::fetching::{Fetching, Mark, account_mark_local, folder_mark};
use dioxus::prelude::*;
use ds::components::content::icon_view::IconView;
use ds::prelude::*;
use ds::style::icon::render::IconSize;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::AccountId;

/// An account's mark.
#[component]
pub(super) fn LinkMark(account: AccountId) -> Element {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return rsx! {};
    };
    let mark = fetching
        .link(account)
        .map_or(Mark::Quiet, |link| account_mark_local(&link));
    drawn(mark)
}

/// A folder's mark.
#[component]
pub(super) fn FolderMark(account: AccountId, path: String) -> Element {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return rsx! {};
    };
    drawn(folder_mark(&fetching.folder(account, &path)))
}

fn drawn(mark: Mark) -> Element {
    match mark {
        Mark::Quiet => rsx! {},
        Mark::Busy => rsx! { Working {} },
        Mark::Warn(why) => rsx! {
            span { class: "fetch-mark", role: "img", "aria-label": "{why}",
                Tooltip { text: why.clone(),
                    IconView { source: IconSource::Glyph(Icon::TriangleAlert), size: IconSize::Compact }
                }
            }
        },
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
