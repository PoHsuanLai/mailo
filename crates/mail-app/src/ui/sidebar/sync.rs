//! The sync mark at the foot's left: what the accounts in view are doing, said where it does not
//! crowd the list.
//!
//! One quiet icon, Sync's arrows. A press fetches what the window shows, and the arrows turn
//! while it works, as Get Mail's do. Its tip says how fresh the mail is, "Sync · 3 min ago",
//! or what is happening or wrong: "Sync · Downloading 3 of 40", "Sync · Not updated since 14:05".
//! A warning also puts a dot on the icon, in the warning's colour, so a stale or broken account
//! is seen without a line of words over the list. With only local folders in view there is
//! nothing to sync, and no mark.
//!
//! While an account in view waits for the person to sign in again, and was put off with Later,
//! the mark is the way back: its tip says Sign In Again, and a press asks about it again in the
//! sign-in prompt instead of syncing.

use crate::ui::copy;
use crate::ui::data::{account_rows, syncs_nothing};
use crate::ui::fetching::{Fetching, StatusLine, Tone, sync_availability, use_status};
use crate::ui::press::on_primary;
use crate::ui::reauth::Reauth;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, BusyLook, ImagePosition};
use ds::prelude::*;
use mail_core::SqliteStore;
use mail_core::fetch::Link;
use std::sync::Arc;

/// The mark's tip: its name, then the line when there is one, in the line's own fewer words
/// for a tip ([`StatusLine::tip`]): "Sync · 3 min ago".
pub(super) fn sync_tip(line: &StatusLine) -> String {
    if line.tip.is_empty() {
        return copy::tooltips::SYNC.to_owned();
    }
    copy::tooltips::sync_saying(&line.tip)
}

/// The mark's whole tip: Sign In Again while a press would ask about a sign-in (`asks`), else
/// what the line says.
pub(super) fn mark_tip(line: Option<&StatusLine>, asks: bool) -> String {
    match line {
        _ if asks => copy::tooltips::SIGN_IN_AGAIN.to_owned(),
        Some(line) => sync_tip(line),
        None => copy::tooltips::SYNC.to_owned(),
    }
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
    if syncs_nothing(&rows(), &shell.read().scope) {
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
    // The accounts in view waiting for a sign-in: a press asks about them again.
    let needing: Vec<porter_core::AccountId> = try_consume_context::<Fetching>()
        .map(|fetching| fetching.needs_in_view(&shell.read()))
        .unwrap_or_default()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let asks = !needing.is_empty();
    let availability = if asks {
        Availability::Enabled
    } else {
        sync_availability(&links.iter().collect::<Vec<&Link>>())
    };
    let tip = mark_tip(line.as_ref(), asks);
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
                label: if asks { copy::aria::SIGN_IN_AGAIN } else { copy::aria::SYNC_NOW },
                icon: Some(IconSource::Glyph(Icon::Refresh)),
                title: Some(tip),
                availability,
                busy: BusyLook::TurnIcon,
                onclick: on_primary(move || {
                    let asked = try_consume_context::<Reauth>()
                        .is_some_and(|reauth| reauth.ask_again(&needing));
                    if !asked {
                        crate::ui::fetching::sync_now(&shell.read());
                    }
                }),
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
            tip: text.to_owned(),
            progress,
            tone,
        }
    }

    #[test]
    fn the_mark_says_the_line_in_its_tip_and_a_warning_with_a_dot() {
        let cases = [
            (line("", Tone::Plain, None), "Sync", None),
            (
                StatusLine {
                    tip: "3 min ago".to_owned(),
                    ..line("Updated 3 minutes ago", Tone::Plain, None)
                },
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
            assert_eq!(mark_tip(Some(&line), false), tip, "{:?}", line.text);
            assert_eq!(dot(&line), warned, "{:?}", line.text);
        }
    }

    /// While an account in view waits for its sign-in, the tip says what a press does.
    #[test]
    fn the_mark_says_sign_in_again_while_a_press_would_ask() {
        let failing = line("Can\u{2019}t fetch mail", Tone::Danger, None);
        assert_eq!(mark_tip(Some(&failing), true), "Sign In Again");
        assert_eq!(mark_tip(None, true), "Sign In Again");
        assert_eq!(mark_tip(None, false), "Sync");
    }
}
