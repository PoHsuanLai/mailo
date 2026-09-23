//! Painting a Space onto `<html>`: once in the head, and again whenever the Space changes.
//!
//! Both paths are built from [`frame_pairs`], so the first frame and a switch cannot disagree
//! about what a Space looks like. Every value inside a script is quoted with `serde_json`: it
//! is an injection site even when it can only be a hex colour or a gradient we computed.

use crate::palette;
use crate::space::{CardAccent, Space};
use crate::view::Theme;

/// How the frame changes to the new Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fade {
    /// The old gradient stays on one layer while the new one fades in on the other.
    Cross,
    /// Straight to the new colours: the editor's live preview, where a fade per drag step
    /// would trail behind the pointer.
    None,
}

/// Every custom property a Space may set inline, so a repaint can clear the last one's.
const NAMES: &[&str] = &[
    "f-ink",
    "f-ink-soft",
    "f-ink-faint",
    "f-pill",
    "f-hover",
    "f-line",
    "f-solid",
    "f-grad",
    "f-grain",
    "accent",
    "accent-soft",
    "accent-ink",
    "f-ink-l",
    "f-ink-soft-l",
    "f-ink-faint-l",
    "f-pill-l",
    "f-hover-l",
    "f-line-l",
    "f-solid-l",
    "f-grad-l",
    "f-grain-l",
    "f-accent-l",
    "f-accent-soft-l",
    "f-accent-ink-l",
    "f-ink-d",
    "f-ink-soft-d",
    "f-ink-faint-d",
    "f-pill-d",
    "f-hover-d",
    "f-line-d",
    "f-solid-d",
    "f-grad-d",
    "f-grain-d",
    "f-accent-d",
    "f-accent-soft-d",
    "f-accent-ink-d",
];

/// Under System with a hint of the Space, both accents exist and the media query picks.
/// Light is the default on `:root`; the media block only switches.
const SYSTEM_ACCENT: &str = ":root:not([data-theme]){--accent:var(--f-accent-l);--accent-soft:var(--f-accent-soft-l);--accent-ink:var(--f-accent-ink-l)}@media (prefers-color-scheme: dark){:root:not([data-theme]){--accent:var(--f-accent-d);--accent-soft:var(--f-accent-soft-d);--accent-ink:var(--f-accent-ink-d)}}";

/// Whether `theme` is the dark frame. System is not a bool: the stylesheet picks, and this
/// returns `None` so both palettes are written instead of guessing the desktop's setting.
fn dark_of(theme: Theme) -> Option<bool> {
    match theme {
        Theme::Light => Some(false),
        Theme::Dark => Some(true),
        Theme::System => None,
    }
}

/// The custom properties `space` sets on `<html>`, as `("--name", value)`, in the order they
/// are written.
///
/// An explicit theme writes the unsuffixed tokens. System writes an `-l` and a `-d` copy of
/// each and lets `tokens.css` choose. The card's accent is written only for
/// [`CardAccent::Hint`]; Postmark is the stylesheet's own.
pub(super) fn frame_pairs(space: &Space) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    match dark_of(space.theme) {
        Some(dark) => {
            push_palette(&mut pairs, space, dark, "");
            if space.card_accent == CardAccent::Hint {
                push_accent(&mut pairs, space, dark, "");
            }
        }
        None => {
            push_palette(&mut pairs, space, false, "-l");
            push_palette(&mut pairs, space, true, "-d");
            if space.card_accent == CardAccent::Hint {
                push_accent(&mut pairs, space, false, "-l");
                push_accent(&mut pairs, space, true, "-d");
            }
        }
    }
    pairs
}

fn push_palette(pairs: &mut Vec<(String, String)>, space: &Space, dark: bool, suffix: &str) {
    let palette = palette::derive(&space.dots, dark);
    let solid = palette.stops.first().cloned().unwrap_or_default();
    let gradient = palette::gradient(&palette);
    let line = if dark {
        "rgba(255,255,255,.09)"
    } else {
        "rgba(0,0,0,.08)"
    };
    let values = [
        ("f-ink", palette.ink),
        ("f-ink-soft", palette.soft),
        ("f-ink-faint", palette.faint),
        ("f-pill", palette.pill),
        ("f-hover", palette.hover),
        ("f-line", line.to_owned()),
        ("f-solid", solid),
        ("f-grad", gradient),
        ("f-grain", grain_opacity(space.grain, dark)),
    ];
    for (name, value) in values {
        pairs.push((format!("--{name}{suffix}"), value));
    }
}

fn push_accent(pairs: &mut Vec<(String, String)>, space: &Space, dark: bool, suffix: &str) {
    let palette = palette::derive(&space.dots, dark);
    // Under System the names are `--f-accent-l`, because `--accent` itself is what the
    // media query assigns. An explicit theme writes `--accent` directly.
    let prefix = if suffix.is_empty() { "" } else { "f-" };
    pairs.push((format!("--{prefix}accent{suffix}"), palette.accent));
    pairs.push((
        format!("--{prefix}accent-soft{suffix}"),
        palette.accent_soft,
    ));
    pairs.push((format!("--{prefix}accent-ink{suffix}"), palette.accent_ink));
}

/// Grain as a CSS opacity: the stored 0–100 times 0.20 in the light and 0.16 in the dark.
fn grain_opacity(grain: u8, dark: bool) -> String {
    let thousandths = i64::from(grain) * if dark { 16 } else { 20 };
    let den = 10_000i64;
    let whole = thousandths / den;
    let frac = thousandths % den;
    if frac == 0 {
        return whole.to_string();
    }
    let mut digits = format!("{frac:04}");
    while digits.ends_with('0') {
        digits.pop();
    }
    format!("{whole}.{digits}")
}

/// `data-motion` and `data-theme`. System deletes the theme attribute rather than setting
/// it: its absence is what lets `prefers-color-scheme` decide, including after a Space that
/// was Light or Dark.
fn look_lines(space: &Space) -> Vec<String> {
    let mut lines = vec![format!(
        "document.documentElement.dataset.motion = {};",
        js(space.motion.slug())
    )];
    lines.push(match space.theme.attribute() {
        Some(theme) => format!("document.documentElement.dataset.theme = {};", js(theme)),
        None => "delete document.documentElement.dataset.theme;".to_owned(),
    });
    lines
}

/// The statements that paint `space`'s tokens, after clearing any earlier Space's.
fn token_lines(space: &Space) -> Vec<String> {
    let mut lines: Vec<String> = NAMES
        .iter()
        .map(|name| {
            format!(
                "document.documentElement.style.removeProperty({});",
                js(&format!("--{name}"))
            )
        })
        .collect();
    lines.extend(
        frame_pairs(space)
            .iter()
            .map(|(name, value)| set(name, value)),
    );
    let rule = match (space.theme, space.card_accent) {
        (Theme::System, CardAccent::Hint) => SYSTEM_ACCENT,
        _ => "",
    };
    lines.push(accent_rule(rule));
    lines
}

/// Put `rule` in the page's own `<style id="mailo-frame">`, after the stylesheet.
///
/// In the head the stylesheet is not in the document yet, so the rule waits for it; at
/// runtime it is, and the rule goes in at once. An empty rule clears a previous Space's.
fn accent_rule(rule: &str) -> String {
    format!(
        "(function () {{\n  var put = function () {{\n    var node = document.getElementById(\"mailo-frame\");\n    if (!node) {{\n      node = document.createElement(\"style\");\n      node.id = \"mailo-frame\";\n      document.documentElement.appendChild(node);\n    }}\n    node.textContent = {};\n  }};\n  if (document.readyState === \"loading\") {{ document.addEventListener(\"DOMContentLoaded\", put); }} else {{ put(); }}\n}})();",
        js(rule)
    )
}

/// The script the head runs before the first frame, and the command menu and the editor's
/// theme segment run again: the look and the tokens, with no fade.
pub(super) fn appearance_script(space: &Space) -> String {
    let mut lines = look_lines(space);
    lines.extend(token_lines(space));
    lines.join("\n")
}

/// The script a switch or a live edit evals.
///
/// [`Fade::Cross`] matches the mockup's `applyPalette(true)`: the gradient on screen is
/// frozen onto the front layer as a literal, the tokens change, and the other layer, which
/// reads `var(--f-grad)`, fades in over 380ms while the front fades out. [`Fade::None`]
/// leaves the front layer reading the token, so the new colour is simply there.
pub(super) fn paint_script(space: &Space, fade: Fade) -> String {
    let before = match fade {
        Fade::Cross => {
            "  if (layers.length === 2) {\n    layers[front].style.background = getComputedStyle(layers[front]).backgroundImage;\n  }"
        }
        Fade::None => {
            "  if (layers.length === 2) {\n    layers[front].style.background = \"\";\n  }"
        }
    };
    let after = match fade {
        Fade::Cross => {
            "  if (layers.length === 2) {\n    var next = 1 - front;\n    layers[next].style.background = \"\";\n    layers[next].style.transition = \"none\";\n    layers[next].style.opacity = \"0\";\n    void layers[next].offsetWidth;\n    layers[next].style.transition = \"\";\n    layers[next].style.opacity = \"1\";\n    layers[front].style.opacity = \"0\";\n    app.dataset.front = String(next);\n  }"
        }
        Fade::None => "",
    };
    let mut body = look_lines(space);
    body.extend(token_lines(space));
    let body = body
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "(function () {{\n  var app = document.querySelector(\".app\");\n  var layers = app ? app.querySelectorAll(\".layer\") : [];\n  var front = app && app.dataset.front === \"1\" ? 1 : 0;\n{before}\n{body}\n{after}\n}})();"
    )
}

fn set(name: &str, value: &str) -> String {
    format!(
        "document.documentElement.style.setProperty({}, {});",
        js(name),
        js(value)
    )
}

fn js(value: &str) -> String {
    serde_json::to_string(value).expect("a &str always serializes") // `&str` serialization cannot fail
}

#[cfg(test)]
mod tests;
