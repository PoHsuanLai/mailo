//! The frame a Space paints, and what the window loads before the first picture.
//!
//! Frame tokens are written by the head script, the same way `data-theme` is: a value
//! inside a `<script>` is quoted with `serde_json`, because it is an injection site even
//! when today it can only be a hex colour or a gradient we computed ourselves.

use crate::appearance::WindowDirs;
use crate::palette;
use crate::space::{self, CardAccent, Scope, Space, Spaces};
use crate::today::{self, Today};
use crate::view::{Appearance, Theme};
use mail_domain::AccountId;
use mail_store::SqliteStore;
use std::sync::Arc;

/// What the first render needs, and nothing it has to ask the disk for again.
#[derive(Clone)]
pub(super) struct Boot {
    pub spaces: Spaces,
    pub today: Today,
    pub dirs: Option<WindowDirs>,
}

/// Load Spaces and Today.
///
/// Called from the component, where the contexts are. A missing file is a first run:
/// one Space per account, saved when there is a config directory to save it in.
pub(super) fn load_boot() -> Boot {
    let dirs = try_consume_dirs();
    let store = dioxus::prelude::consume_context::<Arc<SqliteStore>>();
    let mut spaces = if let Some(dirs) = &dirs {
        space::load(&dirs.config)
    } else {
        dioxus::prelude::try_consume_context::<Spaces>().unwrap_or_default()
    };
    let ids = super::data::accounts(&store);
    if spaces.spaces.is_empty() {
        spaces = space::first_run(&ids);
        if let Some(dirs) = &dirs {
            let _ = space::save(&dirs.config, &spaces);
        }
    }
    let filled = spaces
        .spaces
        .get_mut(spaces.current)
        .is_some_and(|space| space::ensure_colors(space, &ids));
    if filled && let Some(dirs) = &dirs {
        let _ = space::save(&dirs.config, &spaces);
    }
    let mut today = dirs
        .as_ref()
        .map(|dirs| today::load(&dirs.state))
        .unwrap_or_default();
    let before = today.entries.len();
    today.prune(chrono::Utc::now());
    if today.entries.len() != before
        && let Some(dirs) = &dirs
    {
        let _ = today::save(&dirs.state, &today);
    }
    Boot {
        spaces,
        today,
        dirs,
    }
}

fn try_consume_dirs() -> Option<WindowDirs> {
    dioxus::prelude::try_consume_context::<WindowDirs>()
}

/// Accounts a Space limits the list to. Empty means every account.
pub(super) fn scope_ids(space: &Space) -> Vec<AccountId> {
    match &space.scope {
        Scope::All => Vec::new(),
        Scope::Accounts(ids) => ids.clone(),
    }
}

/// Whether `theme` is the dark frame.
///
/// System is not a bool: the stylesheet picks, and this returns `None` so the script
/// writes both palettes instead of guessing what the desktop is set to.
fn dark_of(theme: Theme) -> Option<bool> {
    match theme {
        Theme::Light => Some(false),
        Theme::Dark => Some(true),
        Theme::System => None,
    }
}

/// The statements that paint `space` onto `<html>`, after the appearance lines.
pub(super) fn frame_statements(look: Appearance, space: &Space) -> String {
    let mut lines = Vec::new();
    // A previous explicit theme left inline values. Clearing them is what lets System
    // fall through to the stylesheet, including the `prefers-color-scheme` guard.
    for name in NAMES {
        lines.push(format!(
            "document.documentElement.style.removeProperty({});",
            js(&format!("--{name}"))
        ));
    }
    match dark_of(look.theme) {
        Some(dark) => {
            push_palette(&mut lines, space, dark, "");
            if space.card_accent == CardAccent::Hint {
                push_accent(&mut lines, space, dark, "");
            }
        }
        None => {
            push_palette(&mut lines, space, false, "-l");
            push_palette(&mut lines, space, true, "-d");
            if space.card_accent == CardAccent::Hint {
                push_accent(&mut lines, space, false, "-l");
                push_accent(&mut lines, space, true, "-d");
                lines.push(SYSTEM_ACCENT.to_owned());
            }
        }
    }
    lines.join("\n")
}

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
];

/// System plus a hint of the Space: both accents exist, and the stylesheet's media
/// query is not in the document yet when the head script runs, so the rule is appended
/// once the page has its own styles. Light is the default on `:root`; the media block
/// only switches.
const SYSTEM_ACCENT: &str = r#"document.addEventListener("DOMContentLoaded", function () {
  var node = document.getElementById("mailo-frame");
  if (!node) {
    node = document.createElement("style");
    node.id = "mailo-frame";
    document.documentElement.appendChild(node);
  }
  node.textContent = ":root:not([data-theme]){--accent:var(--f-accent-l);--accent-soft:var(--f-accent-soft-l);--accent-ink:var(--f-accent-ink-l)}@media (prefers-color-scheme: dark){:root:not([data-theme]){--accent:var(--f-accent-d);--accent-soft:var(--f-accent-soft-d);--accent-ink:var(--f-accent-ink-d)}}";
});"#;

fn push_palette(lines: &mut Vec<String>, space: &Space, dark: bool, suffix: &str) {
    let palette = palette::derive(&space.dots, dark);
    let solid = palette.stops.first().cloned().unwrap_or_default();
    let gradient = palette::gradient(&palette);
    let grain = grain_opacity(space.grain, dark);
    let line = if dark {
        "rgba(255,255,255,.09)"
    } else {
        "rgba(0,0,0,.08)"
    };
    let pairs = [
        ("f-ink", palette.ink.as_str()),
        ("f-ink-soft", palette.soft.as_str()),
        ("f-ink-faint", palette.faint.as_str()),
        ("f-pill", palette.pill.as_str()),
        ("f-hover", palette.hover.as_str()),
        ("f-line", line),
        ("f-solid", solid.as_str()),
        ("f-grad", gradient.as_str()),
        ("f-grain", grain.as_str()),
    ];
    for (name, value) in pairs {
        lines.push(set(&format!("--{name}{suffix}"), value));
    }
}

fn push_accent(lines: &mut Vec<String>, space: &Space, dark: bool, suffix: &str) {
    let palette = palette::derive(&space.dots, dark);
    // Under System the names are `--f-accent-l`, because `--accent` itself is what the
    // media query assigns. An explicit theme writes `--accent` directly.
    let (accent, soft, ink) = if suffix.is_empty() {
        ("accent", "accent-soft", "accent-ink")
    } else {
        ("f-accent", "f-accent-soft", "f-accent-ink")
    };
    lines.push(set(&format!("--{accent}{suffix}"), &palette.accent));
    lines.push(set(&format!("--{soft}{suffix}"), &palette.accent_soft));
    lines.push(set(&format!("--{ink}{suffix}"), &palette.accent_ink));
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
