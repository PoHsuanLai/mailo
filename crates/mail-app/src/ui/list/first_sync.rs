//! What the list holds while an account's first mail is on its way: rows in outline, and one
//! line of words under them, so the pane reads as "coming" and not as "empty".

use crate::ui::fetching::FIRST_SYNC;
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::prelude::*;

/// How many placeholder rows fill the pane: more than a pane shows.
const ROWS: usize = 8;

#[component]
pub(super) fn FirstSyncRows() -> Element {
    rsx! {
        div { class: "first-sync",
            Label { text: FIRST_SYNC, role: LabelRole::Tertiary, style: LabelStyle::Caption }
            for n in 0..ROWS {
                SkeletonRow {
                    key: "{n}",
                    // The same two-line rows a thread is, one in three with a short title.
                    lines: if n % 3 == 2 { SkeletonLines::One } else { SkeletonLines::Two },
                }
            }
        }
    }
}
