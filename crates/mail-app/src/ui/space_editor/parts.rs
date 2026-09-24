//! The editor's smaller controls: a segment, the stops, the presets, the readout and the
//! provider marks.

use super::super::icon::{Glyph, Icon};
use super::change;
use crate::appearance::WindowDirs;
use crate::palette;
use crate::space::edit::{Draft, MOST_DOTS};
use crate::space::{PRESET_NAMES, PRESETS, Space, Spaces};
use crate::view::{Appearance, Marks as MarksKind, Shell};
use dioxus::prelude::*;

/// A row of mutually exclusive buttons, each saying with `aria-pressed` whether it is the one.
#[component]
pub(in crate::ui) fn Seg(
    label: String,
    options: Vec<(String, bool)>,
    on_pick: EventHandler<usize>,
) -> Element {
    rsx! {
        div { class: "seg", role: "group", aria_label: "{label}",
            for (index, (name, on)) in options.into_iter().enumerate() {
                button {
                    key: "{index}",
                    r#type: "button",
                    "data-v": "{name}",
                    aria_pressed: if on { "true" } else { "false" },
                    onclick: move |_| on_pick.call(index),
                    "{name}"
                }
            }
        }
    }
}

/// One chip per dot, with its hue and a remove button, then "+ Colour" while there is room.
#[component]
pub(super) fn Stops(editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) -> Element {
    let Some(draft) = editing.read().clone() else {
        return rsx! {};
    };
    let picked = palette::derive(&draft.space.dots, false).picked;
    let several = draft.space.dots.len() > 1;
    let room = draft.space.dots.len() < MOST_DOTS;
    rsx! {
        div { class: "stops",
            for (index, dot) in draft.space.dots.iter().enumerate() {
                {
                    let fill = picked.get(index).cloned().unwrap_or_default();
                    let hue = dot.hue.round();
                    let on = index == draft.active;
                    rsx! {
                        span {
                            key: "{index}",
                            class: "stop",
                            role: "button",
                            aria_pressed: if on { "true" } else { "false" },
                            onclick: move |_| change(editing, spaces, |draft| draft.active = index),
                            i { style: "background:{fill}" }
                            "{hue}°"
                            if several {
                                button {
                                    class: "rm",
                                    r#type: "button",
                                    aria_label: "Remove colour {index + 1}",
                                    onclick: move |event| {
                                        event.stop_propagation();
                                        change(editing, spaces, |draft| {
                                            let _ = draft.remove(index);
                                        });
                                    },
                                    Glyph { icon: Icon::X, class: None }
                                }
                            }
                        }
                    }
                }
            }
            if room {
                button {
                    class: "mini",
                    r#type: "button",
                    onclick: move |_| change(editing, spaces, |draft| {
                        let _ = draft.add();
                    }),
                    Glyph { icon: Icon::Plus, class: None }
                    "Colour"
                }
            }
        }
    }
}

/// The presets, each a swatch of its own gradient, the six retired accents among them.
#[component]
pub(super) fn Presets(editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) -> Element {
    rsx! {
        div { class: "presets", role: "group", aria_label: "Presets",
            for (index, dots) in PRESETS.iter().enumerate() {
                {
                    let grad = super::both_gradients(dots);
                    let name = PRESET_NAMES.get(index).copied().unwrap_or("Preset");
                    rsx! {
                        button {
                            key: "{index}",
                            r#type: "button",
                            aria_label: "{name}",
                            title: "{name}",
                            style: "{grad}",
                            onclick: move |_| change(editing, spaces, |draft| draft.preset(index)),
                        }
                    }
                }
            }
        }
    }
}

/// What `palette::readout` measured, one row per pair, in one theme.
#[component]
pub(super) fn Readout(space: Space, dark: bool, heading: String) -> Element {
    let checks = palette::readout(&space, dark);
    let capped = palette::derive(&space.dots, dark).capped;
    let note = if capped {
        "Chroma was lowered on at least one stop so the text above passes. Drop the dot lower on the field to see the uncapped colour."
    } else {
        "No capping needed: every stop passes at the chroma you chose."
    };
    rsx! {
        div { class: "checks",
            if !heading.is_empty() {
                div { class: "check-h", "{heading}" }
            }
            for check in checks {
                {
                    let passes = check.passes();
                    let measured = format!("{:.2}", check.measured);
                    let need = format!("{:.1}", check.need);
                    rsx! {
                        div { key: "{check.label}", class: "check",
                            span { "{check.label}" }
                            span { class: "v", "{measured}" }
                            span { class: if passes { "pill ok" } else { "pill bad" },
                                if passes { "≥ {need}" } else { "< {need}" }
                            }
                        }
                    }
                }
            }
            p { class: "capnote", "{note}" }
        }
    }
}

/// Provider marks: the window's, not the Space's, so a choice here is kept at once.
#[component]
pub(super) fn Marks(shell: Signal<Shell>) -> Element {
    let now = shell.read().appearance.marks;
    rsx! {
        div {
            div { class: "ed-label", "Provider marks" }
            div { class: "seg", role: "group", aria_label: "Provider marks",
                for marks in MarksKind::ALL {
                    button {
                        key: "{marks.label()}",
                        r#type: "button",
                        aria_pressed: if now == marks { "true" } else { "false" },
                        onclick: move |_| {
                            let look = Appearance { marks, ..shell.read().appearance };
                            shell.write().appearance = look;
                            if let Some(dirs) = try_consume_context::<WindowDirs>() {
                                let _ = crate::appearance::save(&dirs.config, look);
                            }
                        },
                        "{marks.label()}"
                    }
                }
            }
            button {
                class: "marks-refresh",
                r#type: "button",
                onclick: move |_| refresh_icons(),
                "Refresh icons"
            }
        }
    }
}

/// Fetch every provider's icon again, then show the new ones.
fn refresh_icons() {
    let store = consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
    let icons = try_consume_context::<Signal<crate::provider::icon::Loaded>>();
    spawn(async move {
        let Some(root) = crate::appearance::cache_dir() else {
            eprintln!("provider icon: no cache directory");
            return;
        };
        let dir = root.join("providers");
        let providers = match crate::provider::icon::providers_of(&store) {
            Ok(providers) => providers,
            Err(err) => {
                eprintln!("provider icon: {err}");
                return;
            }
        };
        let results = crate::provider::icon::refresh(&dir, &providers).await;
        for (provider, result) in &results {
            if let Err(err) = result
                && !matches!(err, crate::provider::icon::IconError::Unmapped)
            {
                eprintln!("provider icon: {provider:?}: {err}");
            }
        }
        if let Some(mut icons) = icons {
            icons.set(crate::provider::icon::Loaded::read(&dir));
        }
    });
}
