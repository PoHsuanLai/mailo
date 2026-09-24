//! The hue × chroma field: hue across, chroma down, and one handle per dot.
//!
//! Drawn as SVG from `ds::swatch`, so the field shows exactly the colours a dot there
//! would pick, in the scheme the window's root resolved to. The pointer lands on one transparent layer over the whole field, so its
//! coordinates are always the field's own; the handles take the keyboard.

use super::change;
use crate::space::Spaces;
use crate::space::edit::{Draft, Nudge, Stride};
use dioxus::prelude::*;
use ds::{Dot, Scheme};

/// The field's size in CSS pixels. `.field` in `shell.css` is exactly this, and the pointer's
/// offset is divided by it: a field drawn at another size would put a dot beside the pointer.
pub(super) const FIELD_W: f64 = 270.0;
/// See [`FIELD_W`].
pub(super) const FIELD_H: f64 = 176.0;

/// The SVG's own grid, the mockup's canvas: one swatch every 18 units of 540 × 352.
const GRID_W: u32 = 540;
const GRID_H: u32 = 352;
const GRID_STEP: u32 = 18;

/// How near the pointer has to land to pick a handle up rather than move the active one.
const GRAB: f64 = 14.0;

/// The swatches behind the handles, in one scheme. Its own component so a drag, which
/// re-renders the editor on every move, does not rebuild circles that have not changed.
#[component]
fn Swatches(scheme: Scheme) -> Element {
    let cells: Vec<(u32, u32, String)> = (0..GRID_H / GRID_STEP + 1)
        .flat_map(|row| (0..GRID_W / GRID_STEP).map(move |col| (col, row)))
        .map(|(col, row)| {
            let x = col * GRID_STEP + GRID_STEP / 2;
            let y = row * GRID_STEP + GRID_STEP / 2;
            let dot = Dot {
                hue: x as f32 / GRID_W as f32 * 360.0,
                chroma: 1.0 - y as f32 / GRID_H as f32,
            };
            (x, y, ds::swatch(dot, scheme))
        })
        .filter(|(_, y, _)| *y < GRID_H)
        .collect();
    rsx! {
        svg {
            class: "field-dots",
            view_box: "0 0 {GRID_W} {GRID_H}",
            preserve_aspect_ratio: "none",
            "aria-hidden": "true",
            for (x, y, fill) in cells {
                circle { key: "{x}-{y}", cx: "{x}", cy: "{y}", r: "5.2", fill: "{fill}" }
            }
        }
    }
}

/// Which dot a press at `(x, y)` on the field takes: the handle under it, or the active one.
fn grabbed(dots: &[Dot], active: usize, x: f64, y: f64) -> usize {
    dots.iter()
        .enumerate()
        .map(|(index, dot)| {
            let hx = f64::from(dot.hue) / 360.0 * FIELD_W;
            let hy = (1.0 - f64::from(dot.chroma)) * FIELD_H;
            (index, (hx - x).hypot(hy - y))
        })
        .filter(|(_, distance)| *distance <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(active, |(index, _)| index)
}

/// Put the keyboard on handle `index`, so the arrow keys move the dot just dragged.
fn focus_handle(index: usize) {
    let selector = format!(".handle[data-i=\"{index}\"]");
    let quoted = serde_json::to_string(&selector).unwrap_or_default();
    dioxus::document::eval(&format!(
        "var h = document.querySelector({quoted}); if (h) {{ h.focus(); }}"
    ));
}

/// The field and its handles, acting on the draft in `editing`.
#[component]
pub(super) fn HueField(editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) -> Element {
    let mut dragging = use_signal(|| None::<usize>);
    let scheme = ds::use_env().scheme;
    let Some(draft) = editing.read().clone() else {
        return rsx! {};
    };
    let picked = ds::derive(&draft.space.look.dots, Scheme::Light).picked;
    let place = move |dot: usize, x: f64, y: f64| {
        change(editing, spaces, |draft| {
            draft.place(dot, (x / FIELD_W) as f32, (y / FIELD_H) as f32);
        });
    };
    rsx! {
        div { class: if dragging().is_some() { "field dragging" } else { "field" },
            // The field in the scheme the window's root resolved, System included.
            Swatches { scheme }
            div {
                class: "field-hit",
                onpointerdown: move |event| {
                    let point = event.element_coordinates();
                    let (dots, active) = match editing.read().as_ref() {
                        Some(draft) => (draft.space.look.dots.clone(), draft.active),
                        None => return,
                    };
                    let dot = grabbed(&dots, active, point.x, point.y);
                    dragging.set(Some(dot));
                    let id = serde_json::to_string(&event.pointer_id()).unwrap_or_default();
                    dioxus::document::eval(&format!(
                        "var f = document.querySelector(\".field-hit\"); if (f) {{ try {{ f.setPointerCapture({id}); }} catch (e) {{}} }}"
                    ));
                    place(dot, point.x, point.y);
                },
                onpointermove: move |event| {
                    if let Some(dot) = dragging() {
                        let point = event.element_coordinates();
                        place(dot, point.x, point.y);
                    }
                },
                onpointerup: move |_| {
                    if let Some(dot) = dragging() {
                        dragging.set(None);
                        focus_handle(dot);
                    }
                },
            }
            for (index, dot) in draft.space.look.dots.iter().enumerate() {
                {
                    let left = f64::from(dot.hue) / 360.0 * 100.0;
                    let top = (1.0 - f64::from(dot.chroma)) * 100.0;
                    let fill = picked.get(index).cloned().unwrap_or_default();
                    let on = index == draft.active;
                    let hue = dot.hue.round();
                    let chroma = (dot.chroma * 100.0).round();
                    rsx! {
                        div {
                            key: "{index}",
                            class: if on { "handle on" } else { "handle" },
                            role: "slider",
                            tabindex: "0",
                            "data-i": "{index}",
                            aria_label: "Colour {index + 1}",
                            aria_valuetext: "hue {hue}°, chroma {chroma}%",
                            style: "left:{left:.3}%;top:{top:.3}%;background:{fill}",
                            onkeydown: move |event| {
                                let Some(way) = Nudge::of_key(&event.key().to_string()) else {
                                    return;
                                };
                                event.prevent_default();
                                event.stop_propagation();
                                let stride = if event.modifiers().shift() {
                                    Stride::Ten
                                } else {
                                    Stride::One
                                };
                                change(editing, spaces, |draft| draft.nudge(index, way, stride));
                            },
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FIELD_H, FIELD_W, grabbed};
    use ds::Dot;

    #[test]
    fn a_press_near_a_handle_takes_it_and_elsewhere_moves_the_active_one() {
        let dots = [
            Dot {
                hue: 0.0,
                chroma: 1.0,
            },
            Dot {
                hue: 180.0,
                chroma: 0.5,
            },
        ];
        let half = (FIELD_W / 2.0, FIELD_H / 2.0);
        const CASES: &[(&str, usize, (f64, f64), usize)] = &[
            ("on the first handle", 1, (0.0, 0.0), 0),
            ("far from both", 1, (FIELD_W, FIELD_H), 1),
            ("far from both, first active", 0, (FIELD_W, FIELD_H), 0),
        ];
        for &(name, active, (x, y), want) in CASES {
            assert_eq!(grabbed(&dots, active, x, y), want, "{name}");
        }
        assert_eq!(
            grabbed(&dots, 0, half.0 + 3.0, half.1 - 3.0),
            1,
            "near the middle"
        );
    }
}
