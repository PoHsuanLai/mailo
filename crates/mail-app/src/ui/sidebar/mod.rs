//! The sidebar on the Space's colour.
//!
//! Account tiles, places, labels, pinned people, Today, and the foot. Writing and
//! fetching moved into the list bar; the look stays here, behind the gear.

mod panes;
mod today;

use self::panes::{AccountTiles, PinnedList, PlaceList, counts};
use self::today::TodayList;
use super::icon::{Glyph, Icon};
use super::launch::appearance_script;
use crate::appearance::WindowDirs;
use crate::space::Spaces;
use crate::today::Today;
use crate::view::{Accent, Appearance, Motion, Shell, Theme};
use dioxus::prelude::*;
use mail_domain::ThreadId;

/// Show `look` now, and remember it when a config directory exists.
///
/// The frame tokens are stamped in the same script, so a theme click repaints the
/// Space rather than only the card. A file that cannot be written changes nothing
/// the user can see.
fn choose(mut shell: Signal<Shell>, look: Appearance, space: &crate::space::Space) {
    shell.write().appearance = look;
    dioxus::document::eval(&appearance_script(look, space));
    if let Some(dir) = crate::appearance::config_dir() {
        let _ = crate::appearance::save(&dir, look);
    }
}

/// The coloured sidebar.
#[component]
pub(super) fn Places(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
    revision: Signal<u64>,
    spaces: Signal<Spaces>,
    today: Signal<Today>,
    dirs: Option<WindowDirs>,
    side_hidden: Signal<bool>,
    just_added: Signal<Option<ThreadId>>,
) -> Element {
    let space = spaces.read().current_space();
    let counted = use_memo(move || {
        let _ = revision();
        let store = consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
        counts(&store, &spaces.read().current_space())
    });
    let mut open = use_signal(|| false);
    let space_index = spaces.read().current;
    let tiles = counted.read().clone();
    rsx! {
        nav { class: "side", aria_label: "Sidebar",
            button {
                class: "cmd",
                onclick: move |_| {
                    dioxus::document::eval("document.querySelector('input.search')?.focus()");
                },
                Glyph { icon: Icon::Search, class: None }
                span { class: "t", "Search or run a command" }
                span { class: "k", "Ctrl T" }
            }
            div { class: "slide",
                AccountTiles { shell, pages, space: space.clone(), counted: tiles.clone() }
                PlaceList { shell, pages, badges }
                PinnedList { shell, pages, space: space.clone(), pins: tiles.pins.clone() }
                TodayList { shell, today, space_index, dirs: dirs.clone(), just_added }
            }
            div { class: "side-foot",
                span { class: "space-name", "{space.name}" }
                for (index, one) in spaces.read().spaces.iter().enumerate() {
                    {
                        let grad = crate::palette::gradient(&crate::palette::derive(
                            &one.dots,
                            false,
                        ));
                        let name = one.name.clone();
                        let current = index == space_index;
                        rsx! {
                            span {
                                key: "{index}",
                                class: "sp",
                                aria_label: "{name} Space",
                                aria_current: if current { "true" } else { "false" },
                                style: "background:{grad}",
                            }
                        }
                    }
                }
                button {
                    class: "foot-btn",
                    aria_label: "Appearance",
                    aria_expanded: if open() { "true" } else { "false" },
                    onclick: move |_| open.set(!open()),
                    Glyph { icon: Icon::Settings, class: None }
                }
                button {
                    class: "foot-btn",
                    aria_label: "Hide sidebar",
                    onclick: move |_| side_hidden.set(!side_hidden()),
                    Glyph { icon: Icon::PanelLeft, class: None }
                }
                div { class: if open() { "appearance open" } else { "appearance" },
                    p { class: "seg-label", "Theme" }
                    div {
                        class: "theme-choice seg",
                        role: "group",
                        aria_label: "Theme",
                        for theme in [Theme::System, Theme::Light, Theme::Dark] {
                            button {
                                key: "{theme.label()}",
                                aria_pressed: if shell.read().appearance.theme == theme { "true" } else { "false" },
                                onclick: move |_| {
                                    let look = shell.read().appearance;
                                    let space = spaces.read().current_space();
                                    choose(shell, Appearance { theme, ..look }, &space);
                                },
                                "{theme.label()}"
                            }
                        }
                    }
                    p { class: "seg-label", "Motion" }
                    div {
                        class: "motion-choice seg",
                        role: "group",
                        aria_label: "Motion",
                        for motion in Motion::ALL {
                            button {
                                key: "{motion.slug()}",
                                aria_pressed: if shell.read().appearance.motion == motion { "true" } else { "false" },
                                onclick: move |_| {
                                    let look = shell.read().appearance;
                                    let space = spaces.read().current_space();
                                    choose(shell, Appearance { motion, ..look }, &space);
                                },
                                "{motion.label()}"
                            }
                        }
                    }
                    p { class: "seg-label", "Decoration" }
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
                                    let space = spaces.read().current_space();
                                    choose(shell, Appearance { accent, ..look }, &space);
                                },
                            }
                        }
                    }
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
