//! The line under the list's title: what the accounts in view are doing, as Mail's "Updated
//! Just Now", and a small bar while a download knows how much there is.
//!
//! A component of its own so that a tick of the clock, or a pass moving from one step to the
//! next, draws this line and not the list.

use crate::ui::common::classed;
use crate::ui::fetching::{Fetching, Tone, thousandths};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use std::time::Duration;

/// How often "5 minutes ago" is read again. A minute is the finest the words go.
const EVERY: Duration = Duration::from_secs(30);

#[component]
pub(super) fn ListStatus(shell: Signal<Shell>) -> Element {
    let mut tick = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            ds::base::time::clock::sleep(EVERY).await;
            tick += 1;
        }
    });
    let _ = tick();
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return rsx! {};
    };
    let line = fetching.status(&shell.read(), crate::ui::clock::now());
    if line.text.is_empty() {
        return rsx! {};
    }
    let class = match line.tone {
        Tone::Plain => "status",
        Tone::Warn => "status warn",
        Tone::Danger => "status bad",
    };
    rsx! {
        // One line, cut short when it must be; the whole of it on hover.
        Label {
            text: line.text.clone(),
            role: LabelRole::Tertiary,
            style: LabelStyle::Footnote,
            common: classed(class),
        }
        if let Some((done, of)) = line.progress {
            div { class: "list-progress",
                ProgressIndicator {
                    style: ProgressStyle::Bar,
                    progress: Progress::Known(Fraction(thousandths(done, of))),
                    size: ControlSize::Small,
                    common: Common { aria_label: Some(line.text), ..Common::default() },
                }
            }
        }
    }
}
