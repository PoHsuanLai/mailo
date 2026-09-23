use crate::space::{self, CardAccent, PRESET_NAMES, PRESETS, Space};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Scripts, Seen, Work, click, dispatching, press, rebuild_into, type_into, work,
};
use crate::ui::paint::appearance_script;
use crate::ui::sidebar::tests::{Button, buttons_in, pressed};
use crate::ui::style::STYLE;
use crate::view::{Motion, Theme};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;

/// The window on the Work Space, with the editor opened from the Space's name.
///
/// The `Work` comes back too: it owns the temporary directories the window writes into.
fn opened() -> (VirtualDom, Seen, Work, Scripts) {
    dispatching();
    let built = work();
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(scripts.document());
    let seen = rebuild_into(&mut dom);
    let name = seen.one("aria-label", "Edit the Work Space");
    let seen = click(&mut dom, name);
    (dom, seen, built, scripts)
}

/// The frame's markup with the editor open, for the stylesheet's class check.
pub(in crate::ui) fn editor_open_markup() -> String {
    let (dom, _, _built, _) = opened();
    dioxus_ssr::render(&dom)
}

/// Each `.seg` group's `aria-label`, and its buttons.
fn segments(page: &str) -> Vec<(String, Vec<Button>)> {
    const OPEN: &str = "class=\"seg\" role=\"group\" aria-label=\"";
    let mut out = Vec::new();
    let mut rest = page;
    while let Some(at) = rest.find(OPEN) {
        let tail = &rest[at + OPEN.len()..];
        let end = tail.find('"').unwrap_or(0);
        let label = tail[..end].to_owned();
        let close = tail.find("</div>").unwrap_or(tail.len());
        let body = format!("<div class=\"seg\" {}</div>", &tail[end + 1..close]);
        out.push((label, buttons_in(&body, "seg")));
        rest = &tail[end..];
    }
    out
}

#[tokio::test]
async fn the_editor_opens_on_the_spaces_own_choices() {
    let (dom, _, _built, _) = opened();
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("aria-label=\"Space editor\""), "{page}");

    let groups = [
        ("Theme", ["System", "Light", "Dark"].as_slice(), "System"),
        (
            "Motion",
            ["Calm", "Standard", "Extra"].as_slice(),
            "Standard",
        ),
        (
            "Card accent",
            ["A hint of the Space", "Postmark"].as_slice(),
            "A hint of the Space",
        ),
        (
            "Provider marks",
            ["Their icons", "Letters"].as_slice(),
            "Their icons",
        ),
    ];
    let segs = segments(&page);
    for (label, names, on) in groups {
        let buttons = segs
            .iter()
            .find(|(name, _)| name == label)
            .map(|(_, buttons)| buttons)
            .unwrap_or_else(|| panic!("no {label} segment in {segs:?}", segs = segs.len()));
        let texts: Vec<&str> = buttons.iter().map(|button| button.text.as_str()).collect();
        assert_eq!(texts, names, "{label}");
        assert_eq!(
            pressed(buttons, |button| button.text.as_str()),
            [on],
            "{label}"
        );
    }

    let presets = buttons_in(&page, "presets");
    let names: Vec<&str> = presets
        .iter()
        .map(|button| button.attr("aria-label"))
        .collect();
    assert_eq!(names, PRESET_NAMES, "{page}");
    assert_eq!(presets.len(), PRESETS.len());
    for hue in [
        "Postmark",
        "Graphite",
        "Pine",
        "Indigo",
        "Oxblood",
        "Vermilion",
    ] {
        assert!(names.contains(&hue), "{hue} is not among the presets");
    }
    assert!(page.contains("Refresh icons"), "{page}");
    assert!(
        page.contains("Sidebar text on the colour"),
        "no readout: {page}"
    );
    assert_eq!(
        page.matches("role=\"slider\"").count(),
        2,
        "the Work Space has two dots: {page}"
    );
    assert!(
        page.contains("class=\"inp range\""),
        "grain is not a Field: {page}"
    );
}

#[tokio::test]
async fn escape_puts_the_space_back_exactly_and_writes_nothing() {
    let (mut dom, seen, built, scripts) = opened();
    let file = built.dirs.config.join("spaces.json");
    let stored = std::fs::read(&file).unwrap_or_else(|e| panic!("{e}"));
    let saved = space::load(&built.dirs.config).current_space();

    let _ = type_into(&mut dom, seen.one("value", "Work"), "Elsewhere");
    let _ = click(&mut dom, seen.one("data-v", "Dark"));
    let edited = dioxus_ssr::render(&dom);
    assert!(
        edited.contains("Edit the Elsewhere Space"),
        "the name did not reach the frame live: {edited}"
    );
    let evals = scripts.all().len();

    press(&mut dom, "Escape", INSIDE_THE_SHELL);
    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("aria-label=\"Space editor\""),
        "Esc left the editor open"
    );
    assert!(
        page.contains("Edit the Work Space"),
        "the name was not put back: {page}"
    );
    assert_eq!(
        std::fs::read(&file).unwrap_or_else(|e| panic!("{e}")),
        stored,
        "Esc wrote spaces.json"
    );
    assert_eq!(space::load(&built.dirs.config).current_space(), saved);
    // And the frame was repainted as the saved Space: System again, not the Dark tried.
    let after = &scripts.all()[evals..];
    assert!(
        after
            .iter()
            .any(|script| script.contains("delete document.documentElement.dataset.theme;")),
        "Esc did not repaint the saved Space: {after:#?}"
    );
}

#[tokio::test]
async fn save_writes_the_space_and_it_reads_back_the_same() {
    let (mut dom, seen, built, scripts) = opened();
    let before = space::load(&built.dirs.config).current_space();
    let evals = scripts.all().len();

    let _ = type_into(&mut dom, seen.one("value", "Work"), "Studio");
    let _ = click(&mut dom, seen.one("data-v", "Dark"));
    let _ = click(&mut dom, seen.one("data-v", "Postmark"));
    let _ = type_into(&mut dom, seen.one("value", "35"), "80");
    let live = scripts.all()[evals..].to_vec();
    let _ = click(&mut dom, seen.one("title", "Save this Space and close"));

    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("aria-label=\"Space editor\""),
        "Save left the editor open"
    );
    let want = Space {
        name: "Studio".to_owned(),
        theme: Theme::Dark,
        card_accent: CardAccent::Postmark,
        grain: 80,
        ..before.clone()
    };
    assert_ne!(before, want, "the edits above changed nothing");
    assert_eq!(space::load(&built.dirs.config).current_space(), want);
    assert_eq!(want.motion, Motion::Standard);
    assert!(
        live.iter()
            .any(|script| script.contains("document.documentElement.dataset.theme = \"dark\";")),
        "choosing Dark did not repaint the frame live: {live:#?}"
    );
}

#[tokio::test]
#[ignore]
async fn render_the_space_editor_to_a_file() {
    let (dom, _, built, _) = opened();
    let body = dioxus_ssr::render(&dom);
    let space = space::load(&built.dirs.config).current_space();
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap_or_else(|e| panic!("{e}"));
    for (suffix, theme) in [("", Theme::Light), ("-dark", Theme::Dark)] {
        let script = appearance_script(&Space {
            theme,
            ..space.clone()
        });
        let theme_attr = theme
            .attribute()
            .map(|name| format!(" data-theme=\"{name}\""))
            .unwrap_or_default();
        let page = format!(
            "<!doctype html>\n<html lang=\"en\"{theme_attr}>\
             <head><meta charset=\"utf-8\"><style>{STYLE}</style><script>{script}</script></head>\
             <body>{body}</body></html>\n"
        );
        let out = target.join(format!("space-editor{suffix}.html"));
        std::fs::write(&out, page).unwrap_or_else(|e| panic!("{e}"));
        println!("wrote {}", out.display());
    }
}
