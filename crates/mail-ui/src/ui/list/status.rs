//! A download that knows how much there is: its bar, across the list's top under the toolbar.
//!
//! What the accounts in view are doing is said by the sync mark at the sidebar's foot, in its
//! tip, with a dot when something is wrong; the list's header holds its title and its tools and
//! no words about syncing. A count is the one thing drawn here as well, because it is the list
//! it is filling. A component of its own, so a tick of the clock draws the bar and not the list.

use crate::ui::fetching::use_status;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::root::common::Common;
use mail_ui_model::fetching::status::thousandths;

#[component]
pub(super) fn ListProgress(shell: Signal<Shell>) -> Element {
    let Some(line) = use_status(shell) else {
        return rsx! {};
    };
    let Some((done, of)) = line.progress else {
        return rsx! {};
    };
    rsx! {
        div { class: "list-progress",
            ProgressIndicator {
                style: ProgressStyle::Bar,
                progress: Progress::Known(Fraction(thousandths(done, of))),
                common: Common { aria_label: Some(line.text), ..Common::default() },
            }
        }
    }
}
