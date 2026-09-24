//! The editor's smaller controls: a segment, the stops, the presets, the readout and the
//! provider marks.

use super::change;
use crate::appearance::WindowDirs;
use crate::space::edit::{Draft, MOST_DOTS};
use crate::space::{PRESET_NAMES, PRESETS, Space, Spaces};
use crate::view::{Appearance, Marks as MarksKind, Shell};
use dioxus::prelude::*;
use ds::{Capping, Glyph, Icon, Scheme, SegmentedControl};

/// A row of mutually exclusive buttons, each saying with `aria-pressed` whether it is the one:
/// quire's `SegmentedControl`, choosing by position. The caller's options say which is on.
#[component]
pub(in crate::ui) fn Seg(
    label: String,
    options: Vec<(String, bool)>,
    on_pick: EventHandler<usize>,
) -> Element {
    let value = options.iter().position(|(_, on)| *on).unwrap_or(usize::MAX);
    let options: Vec<(usize, String)> = options
        .into_iter()
        .enumerate()
        .map(|(index, (name, _))| (index, name))
        .collect();
    rsx! {
        SegmentedControl::<usize> { label, options, value, onchange: move |index| on_pick.call(index) }
    }
}

/// One chip per dot, with its hue and a remove button, then "+ Colour" while there is room.
#[component]
pub(super) fn Stops(editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) -> Element {
    let Some(draft) = editing.read().clone() else {
        return rsx! {};
    };
    let picked = ds::derive(&draft.space.look.dots, Scheme::Light).picked;
    let several = draft.space.look.dots.len() > 1;
    let room = draft.space.look.dots.len() < MOST_DOTS;
    rsx! {
        div { class: "stops",
            for (index, dot) in draft.space.look.dots.iter().enumerate() {
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
                                    Glyph { icon: Icon::X, size: ds::IconSize::Micro }
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
                    Glyph { icon: Icon::Plus, size: ds::IconSize::Tiny }
                    "Colour"
                }
            }
        }
    }
}

/// The presets, each a swatch of its own gradient, the six retired accents among them.
#[component]
pub(super) fn Presets(editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) -> Element {
    let scheme = ds::use_env().scheme;
    rsx! {
        div { class: "presets", role: "group", aria_label: "Presets",
            for (index, dots) in PRESETS.iter().enumerate() {
                {
                    let grad = super::gradient_in(dots, scheme);
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

/// What `ds::readout` measured, one row per pair, in one scheme.
#[component]
pub(super) fn Readout(space: Space, scheme: Scheme, heading: String) -> Element {
    let checks = ds::readout(&space.look, scheme);
    let capped = ds::derive(&space.look.dots, scheme).capped == Capping::Capped;
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
                    let passes = check.verdict() == ds::Verdict::Pass;
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
            SegmentedControl::<MarksKind> {
                label: "Provider marks",
                options: MarksKind::ALL.into_iter().map(|marks| (marks, marks.label().to_owned())).collect::<Vec<_>>(),
                value: now,
                onchange: move |marks| {
                    let look = Appearance { marks };
                    shell.write().appearance = look;
                    if let Some(dirs) = try_consume_context::<WindowDirs>() {
                        let _ = crate::appearance::save(&dirs.config, look);
                    }
                },
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
