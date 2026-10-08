//! What the accounts in view are doing. All is well: no words under the title, only the Sync
//! button's tip, "Updated 3 minutes ago", and its arrows turning while it works. Words appear
//! when there is something to read: a download that knows how much there is, with its bar, or a
//! warning, which opens the Connection Doctor.
//!
//! Components of their own so that a tick of the clock, or a pass moving from one step to the
//! next, draws the line and the button and not the list.

use crate::ui::common::classed;
use crate::ui::fetching::{Fetching, StatusLine, Tone, thousandths};
use crate::ui::press::on_primary;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::components::controls::button_model::{Bezel, BusyLook, ImagePosition};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use std::time::Duration;

/// How often "5 minutes ago" is read again. A minute is the finest the words go.
const EVERY: Duration = Duration::from_secs(30);

/// Where a status line is said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Said {
    /// Nothing to say.
    Nowhere,
    /// All is well: in the Sync button's tip, not on the page.
    InTip,
    /// Under the title: a count with its bar, or a warning.
    OnLine,
}

/// Where `line` belongs.
pub(super) fn said(line: &StatusLine) -> Said {
    match (line.text.is_empty(), line.tone, line.progress) {
        (true, _, _) => Said::Nowhere,
        (false, Tone::Plain, None) => Said::InTip,
        _ => Said::OnLine,
    }
}

/// The Sync button's tip: what it does, and how fresh the mail is when all is well.
pub(super) fn sync_tip(line: &StatusLine) -> String {
    match said(line) {
        Said::InTip => format!("Sync now \u{b7} {}", line.text),
        Said::Nowhere | Said::OnLine => "Sync now".to_owned(),
    }
}

/// The line as it stands, read again every [`EVERY`] so "5 minutes ago" moves on.
fn use_line(shell: Signal<Shell>) -> Option<StatusLine> {
    let mut tick = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            ds::base::time::clock::sleep(EVERY).await;
            tick += 1;
        }
    });
    let _ = tick();
    let fetching = try_consume_context::<Fetching>()?;
    Some(fetching.status(&shell.read(), crate::ui::clock::now()))
}

/// The toolbar's Sync: its arrows turn while it works, as Get Mail's do, and its tip says when
/// the mail was last fetched.
#[component]
pub(super) fn SyncButton(shell: Signal<Shell>, availability: Availability) -> Element {
    let tip = use_line(shell).map_or_else(|| "Sync now".to_owned(), |line| sync_tip(&line));
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            label: "Sync now",
            icon: Some(IconSource::Glyph(Icon::Refresh)),
            title: Some(tip),
            availability,
            busy: BusyLook::TurnIcon,
            onclick: on_primary(move || crate::ui::fetching::sync_now(&shell.read())),
        }
    }
}

#[component]
pub(super) fn ListStatus(shell: Signal<Shell>) -> Element {
    let Some(line) = use_line(shell) else {
        return rsx! {};
    };
    if said(&line) != Said::OnLine {
        return rsx! {};
    }
    let class = match line.tone {
        Tone::Plain => "status",
        Tone::Warn => "status warn",
        Tone::Danger => "status bad",
    };
    // A warning is a way in to the Connection Doctor, as Mail's mark is; a quiet line is words.
    let words = match line.tone {
        Tone::Plain => rsx! {
            // A count, cut short when it must be; the whole of it on hover.
            Label {
                text: line.text.clone(),
                role: LabelRole::Tertiary,
                style: LabelStyle::Footnote,
                common: classed(class),
            }
        },
        Tone::Warn | Tone::Danger => rsx! {
            Button {
                label: line.text.clone(),
                bezel: Bezel::Inline,
                title: Some("Open Connection Doctor".to_owned()),
                onclick: on_primary(move || crate::ui::doctor::open(shell)),
                // `data-opens`: what it opens, for a test to find it by, since its words change.
                common: Common {
                    extra_class: classed(class).extra_class,
                    ..crate::ui::sidebar::tagged("opens", "doctor")
                },
            }
        },
    };
    rsx! {
        {words}
        if let Some((done, of)) = line.progress {
            div { class: "list-progress",
                ProgressIndicator {
                    style: ProgressStyle::Bar,
                    progress: Progress::Known(Fraction(thousandths(done, of))),
                    common: Common { aria_label: Some(line.text), ..Common::default() },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, tone: Tone, progress: Option<(u32, u32)>) -> StatusLine {
        StatusLine {
            text: text.to_owned(),
            progress,
            tone,
        }
    }

    #[test]
    fn a_line_is_said_where_it_is_worth_reading() {
        let cases = [
            (line("", Tone::Plain, None), Said::Nowhere, "Sync now"),
            (
                line("Updated 3 minutes ago", Tone::Plain, None),
                Said::InTip,
                "Sync now \u{b7} Updated 3 minutes ago",
            ),
            (
                line("Downloading 3 of 40", Tone::Plain, Some((3, 40))),
                Said::OnLine,
                "Sync now",
            ),
            (line("Offline", Tone::Warn, None), Said::OnLine, "Sync now"),
            (
                line("Sign-in failed", Tone::Danger, None),
                Said::OnLine,
                "Sync now",
            ),
        ];
        for (line, where_, tip) in cases {
            assert_eq!(said(&line), where_, "{:?}", line.text);
            assert_eq!(sync_tip(&line), tip, "{:?}", line.text);
        }
    }
}
