//! What the list holds while an account's first mail is on its way: rows in outline, so the pane
//! reads as "coming" and not as "empty". No words: the counted download's bar across the list's
//! top says how far it has got.

use dioxus::prelude::*;
use ds::prelude::*;

/// How many placeholder rows fill the pane: more than a pane shows.
const ROWS: usize = 8;

#[component]
pub(super) fn FirstSyncRows() -> Element {
    rsx! {
        div { class: "first-sync",
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
