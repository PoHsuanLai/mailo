//! The sync mark at the foot's left: what the accounts in view are doing, said where it does not
//! crowd the list.
//!
//! One quiet icon, Sync's arrows. A press fetches what the window shows, and the arrows turn
//! while it works, as Get Mail's do. Its tip says how fresh the mail is, "Sync · 3 min ago",
//! or what is happening or wrong: "Sync · Downloading 3 of 40", "Sync · Not updated since 14:05".
//! A warning also puts a dot on the icon, in the warning's colour, so a stale or broken account
//! is seen without a line of words over the list. With only local folders in view there is
//! nothing to sync, and no mark.

use crate::ui::data::{account_rows, syncs_nothing};
use crate::ui::fetching::{Fetching, StatusLine, Tone, sync_availability, use_status};
use crate::ui::press::on_primary;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, BusyLook, ImagePosition};
use ds::prelude::*;
use mail_core::SqliteStore;
use mail_core::fetch::Link;
use std::sync::Arc;

/// The mark's tip: its name, then the line when there is one, in as few words as say it:
/// "Sync · 3 min ago".
pub(super) fn sync_tip(line: &StatusLine) -> String {
    if line.text.is_empty() {
        return "Sync".to_owned();
    }
    format!("Sync \u{b7} {}", fresh(&line.text))
}

/// "Updated 3 minutes ago" as a tip says it: "3 min ago". Any other line is said as it is.
fn fresh(text: &str) -> String {
    text.strip_prefix("Updated ")
        .unwrap_or(text)
        .replace(" minutes ago", " min ago")
        .replace(" minute ago", " min ago")
}

/// The class of the dot a line puts on the mark: none when all is well or busy, else the
/// warning's.
pub(super) fn dot(line: &StatusLine) -> Option<&'static str> {
    match line.tone {
        Tone::Plain => None,
        Tone::Warn => Some("sync-dot"),
        Tone::Danger => Some("sync-dot bad"),
    }
}

/// The foot's sync mark, for the accounts `shell` shows.
#[component]
pub(super) fn SyncMark(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let rows = use_memo(move || {
        let _ = revision();
        try_consume_context::<Arc<SqliteStore>>()
            .map(|store| account_rows(&store))
            .unwrap_or_default()
    });
    let line = use_status(shell);
    // Local folders are never synced: with only them in view there is no Sync, and no word of one.
    if syncs_nothing(&rows(), shell.read().account.clone(), &shell.read().scope) {
        return rsx! {};
    }
    let links: Vec<Link> = try_consume_context::<Fetching>()
        .map(|fetching| {
            fetching
                .links_in_view(&shell.read())
                .into_iter()
                .map(|(_, link)| link)
                .collect()
        })
        .unwrap_or_default();
    let availability = sync_availability(&links.iter().collect::<Vec<&Link>>());
    let tip = line.as_ref().map_or_else(|| "Sync".to_owned(), sync_tip);
    let warned = line.as_ref().and_then(dot);
    rsx! {
        span { class: "sync-hold",
            // The foot's right click is the Space's menu; here it is nobody's.
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
            },
            Button {
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                label: "Sync now",
                icon: Some(IconSource::Glyph(Icon::Refresh)),
                title: Some(tip),
                availability,
                busy: BusyLook::TurnIcon,
                onclick: on_primary(move || crate::ui::fetching::sync_now(&shell.read())),
            }
            if let Some(class) = warned {
                span { class, aria_hidden: "true" }
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
    fn the_mark_says_the_line_in_its_tip_and_a_warning_with_a_dot() {
        let cases = [
            (line("", Tone::Plain, None), "Sync", None),
            (
                line("Updated 3 minutes ago", Tone::Plain, None),
                "Sync \u{b7} 3 min ago",
                None,
            ),
            (
                line("Downloading 3 of 40", Tone::Plain, Some((3, 40))),
                "Sync \u{b7} Downloading 3 of 40",
                None,
            ),
            (
                line("Not updated since 14:05", Tone::Warn, None),
                "Sync \u{b7} Not updated since 14:05",
                Some("sync-dot"),
            ),
            (
                line("Offline \u{b7} trying again at 14:05", Tone::Warn, None),
                "Sync \u{b7} Offline \u{b7} trying again at 14:05",
                Some("sync-dot"),
            ),
            (
                line("Can\u{2019}t fetch mail", Tone::Danger, None),
                "Sync \u{b7} Can\u{2019}t fetch mail",
                Some("sync-dot bad"),
            ),
        ];
        for (line, tip, warned) in cases {
            assert_eq!(sync_tip(&line), tip, "{:?}", line.text);
            assert_eq!(dot(&line), warned, "{:?}", line.text);
        }
    }
}
