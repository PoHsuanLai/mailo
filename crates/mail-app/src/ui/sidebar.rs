//! The places down the side of the window.
//!
//! Where a conversation can be, plus the two actions that are not about one conversation:
//! writing a new message, and fetching mail. Split from [`super::app`] (`CONVENTIONS.md` §8).
//! The hooks that feed the badges stay in `App`; this only reads the memo it is handed, so a
//! keystroke in the search box does not recount them.

use super::launch::appearance_script;
use super::ops::start_new;
use crate::view::{Accent, Appearance, Motion, Shell, SyncState, Theme, synced};
use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;

/// Show `look` now, and remember it when a config directory exists.
///
/// A file that cannot be written changes nothing the user can see: the window already
/// wears `look`, and a cosmetic miss is not a reason to unwind the click.
fn choose(mut shell: Signal<Shell>, look: Appearance) {
    shell.write().appearance = look;
    dioxus::document::eval(&appearance_script(look));
    if let Some(dir) = crate::appearance::config_dir() {
        let _ = crate::appearance::save(&dir, look);
    }
}

/// The places a conversation can be, and the way to write or fetch.
#[component]
pub(super) fn Places(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
    revision: Signal<u64>,
    sync_state: Signal<SyncState>,
) -> Element {
    rsx! {
        nav { class: "places",
            for (index, place) in shell.read().places.iter().enumerate() {
                button {
                    key: "{place.name}",
                    class: if index == shell.read().selected { "place on" } else { "place" },
                    onclick: move |_| {
                        shell.write().select(index);
                        pages.set(1);
                    },
                    "{place.name}"
                    if let Some(Some(count)) = badges().get(index).copied() {
                        span { class: "badge", "{count}" }
                    }
                }
            }
            button {
                class: "place compose",
                onclick: move |_| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    let known = shell.peek().accounts.clone();
                    match start_new(&store, &known) {
                        Ok(draft) => {
                            shell.write().compose(&draft);
                            revision += 1;
                        }
                        Err(why) => eprintln!("compose: {why}"),
                    }
                },
                title: "Write a new message (c)",
                "New"
            }
            div { class: "spacer" }
            div { class: "appearance",
                div {
                    class: "theme-choice",
                    role: "group",
                    aria_label: "Theme",
                    for theme in [Theme::System, Theme::Light, Theme::Dark] {
                        button {
                            key: "{theme.label()}",
                            aria_pressed: if shell.read().appearance.theme == theme { "true" } else { "false" },
                            onclick: move |_| {
                                let look = shell.read().appearance;
                                choose(shell, Appearance { theme, ..look });
                            },
                            "{theme.label()}"
                        }
                    }
                }
                div {
                    class: "motion-choice",
                    role: "group",
                    aria_label: "Motion",
                    for motion in Motion::ALL {
                        button {
                            key: "{motion.slug()}",
                            aria_pressed: if shell.read().appearance.motion == motion { "true" } else { "false" },
                            onclick: move |_| {
                                let look = shell.read().appearance;
                                choose(shell, Appearance { motion, ..look });
                            },
                            "{motion.label()}"
                        }
                    }
                }
                div {
                    class: "accent-choice",
                    role: "group",
                    aria_label: "Decoration",
                    for accent in Accent::ALL {
                        button {
                            key: "{accent.slug()}",
                            class: "swatch",
                            "data-hue": "{accent.slug()}",
                            aria_label: "{accent.label()}",
                            title: "{accent.label()}",
                            aria_pressed: if shell.read().appearance.accent == accent { "true" } else { "false" },
                            onclick: move |_| {
                                let look = shell.read().appearance;
                                choose(shell, Appearance { accent, ..look });
                            },
                        }
                    }
                }
            }
            button {
                class: "place sync",
                disabled: !sync_state.read().may_start(),
                onclick: move |_| {
                    if !sync_state.read().may_start() {
                        return;
                    }
                    sync_state.set(SyncState::Running);
                    let store = consume_context::<Arc<SqliteStore>>();
                    spawn(async move {
                        // `spawn_blocking`, not this task: sync::run opens sockets and
                        // builds its own runtime, and `Runtime::block_on` inside an async
                        // context panics. Off the UI thread either way — a pass takes
                        // minutes on a first sync and would freeze the window.
                        let done = tokio::task::spawn_blocking(move || {
                            crate::sync::run(store, chrono::Utc::now())
                        })
                        .await;
                        sync_state.set(match done {
                            Ok(result) => synced(result.map(|ran| ran.text)),
                            // The blocking task panicked. Saying so beats a window that
                            // sits on "Syncing…" for ever.
                            Err(e) => synced(Err(format!("the sync pass stopped: {e}"))),
                        });
                        revision += 1;
                    });
                },
                if sync_state.read().may_start() { "Sync" } else { "Syncing…" }
            }
            if let Some(note) = sync_state.read().message() {
                p {
                    class: if sync_state.read().is_failure() { "sync-note bad" } else { "sync-note" },
                    "{note}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::App;
    use crate::ui::fixtures::empty;
    use crate::view::{Accent, Appearance, Motion, Theme};
    use dioxus::prelude::*;

    #[tokio::test]
    async fn the_picker_marks_a_dark_pine_window() {
        let (store, _dir) = empty();
        let look = Appearance {
            theme: Theme::Dark,
            accent: Accent::Pine,
            motion: Motion::Calm,
        };
        let mut dom = VirtualDom::new(App)
            .with_root_context(store)
            .with_root_context(look);
        dom.rebuild_in_place();
        let page = dioxus_ssr::render(&dom);

        let themes = buttons_in(&page, "theme-choice");
        let swatches = buttons_in(&page, "accent-choice");
        let motions = buttons_in(&page, "motion-choice");
        assert_eq!(
            themes
                .iter()
                .map(|button| button.text.as_str())
                .collect::<Vec<_>>(),
            ["System", "Light", "Dark"],
            "{page}"
        );
        let hues: Vec<&str> = swatches
            .iter()
            .map(|button| button.attr("data-hue"))
            .collect();
        let expect: Vec<&str> = Accent::ALL.iter().map(|hue| hue.slug()).collect();
        assert_eq!(hues, expect, "{page}");
        assert_eq!(
            pressed(&themes, |button| button.text.as_str()),
            ["Dark"],
            "{page}"
        );
        assert_eq!(
            pressed(&swatches, |button| button.attr("data-hue")),
            ["pine"],
            "{page}"
        );
        assert_eq!(
            pressed(&motions, |button| button.text.as_str()),
            ["Calm"],
            "{page}"
        );
    }

    struct Button {
        attrs: Vec<(String, String)>,
        text: String,
    }

    impl Button {
        fn attr(&self, name: &str) -> &str {
            self.attrs
                .iter()
                .find(|(got, _)| got == name)
                .map(|(_, value)| value.as_str())
                .unwrap_or("")
        }
    }

    /// The identity of every button whose `aria-pressed` is `true`.
    /// Every button in the group must say true or false, so a missing attribute fails here
    /// rather than looking like "nothing is pressed".
    fn pressed<'a>(buttons: &'a [Button], id: impl Fn(&'a Button) -> &'a str) -> Vec<&'a str> {
        let mut on = Vec::new();
        for button in buttons {
            let state = button.attr("aria-pressed");
            assert!(
                state == "true" || state == "false",
                "aria-pressed is {state:?} on {}",
                id(button)
            );
            if state == "true" {
                on.push(id(button));
            }
        }
        on
    }

    /// The `<button>` elements inside the element whose class list contains `class`.
    fn buttons_in(html: &str, class: &str) -> Vec<Button> {
        let mut buttons = Vec::new();
        let mut group_depth: Option<usize> = None;
        let mut depth = 0usize;
        let mut index = 0usize;
        let bytes = html.as_bytes();
        while index < bytes.len() {
            if bytes[index] != b'<' {
                index += 1;
                continue;
            }
            let rest = &html[index..];
            if rest.starts_with("</") {
                let Some(end) = rest.find('>') else { break };
                if group_depth == Some(depth) {
                    group_depth = None;
                }
                depth = depth.saturating_sub(1);
                index += end + 1;
                continue;
            }
            if rest.starts_with("<!") || rest.starts_with("<?") {
                let Some(end) = rest.find('>') else { break };
                index += end + 1;
                continue;
            }
            let Some(end) = rest.find('>') else { break };
            let raw = &rest[1..end];
            let self_closing = raw.ends_with('/');
            let raw = raw.trim_end_matches('/').trim();
            let name = raw.split_whitespace().next().unwrap_or("");
            let attrs = attributes(raw);
            let in_group = group_depth.is_some();
            if !self_closing {
                depth += 1;
                if group_depth.is_none() && has_class(&attrs, class) {
                    group_depth = Some(depth);
                }
            }
            if in_group && name == "button" {
                let text = if self_closing {
                    String::new()
                } else {
                    let after = index + end + 1;
                    let close = html[after..].find("</button>").unwrap_or(0);
                    html[after..after + close].trim().to_string()
                };
                buttons.push(Button { attrs, text });
            }
            index += end + 1;
        }
        buttons
    }

    fn has_class(attrs: &[(String, String)], class: &str) -> bool {
        attrs.iter().any(|(name, value)| {
            name == "class" && value.split_whitespace().any(|token| token == class)
        })
    }

    fn attributes(raw: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut rest = match raw.find(char::is_whitespace) {
            Some(space) => raw[space..].trim_start(),
            None => return out,
        };
        while !rest.is_empty() {
            let name_end = rest.find(['=', ' ', '\n', '\t']).unwrap_or(rest.len());
            let name = &rest[..name_end];
            rest = rest[name_end..].trim_start();
            if name.is_empty() {
                continue;
            }
            if !rest.starts_with('=') {
                out.push((name.to_string(), String::new()));
                continue;
            }
            rest = rest[1..].trim_start();
            if rest.is_empty() {
                out.push((name.to_string(), String::new()));
                break;
            }
            let quote = rest.as_bytes()[0];
            if quote == b'"' || quote == b'\'' {
                rest = &rest[1..];
                let value_end = rest.find(quote as char).unwrap_or(rest.len());
                out.push((name.to_string(), rest[..value_end].to_string()));
                rest = rest.get(value_end + 1..).unwrap_or("").trim_start();
            } else {
                let value_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                out.push((name.to_string(), rest[..value_end].to_string()));
                rest = rest[value_end..].trim_start();
            }
        }
        out
    }
}
