use crate::space::{self, PRESET_NAMES, PRESETS, Space};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Scripts, Seen, Work, click, dispatching, press, rebuild_into, root_attr,
    type_into, work,
};
use crate::ui::sidebar::tests::{Button, buttons_in, pressed};
use crate::view::{Motion, Theme};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use ds::{CardAccent, Grain, SpaceLook};

/// The window on the Work Space, with the editor opened from the Space's name.
///
/// The `Work` comes back too: it owns the temporary directories the window writes into.
fn opened() -> (VirtualDom, Seen, Work, Scripts) {
    opened_with(ds_settings::Environment::default())
}

/// The editor open on the Work Space, in a window reading `environment`.
fn opened_with(environment: ds_settings::Environment) -> (VirtualDom, Seen, Work, Scripts) {
    dispatching();
    let built = work();
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone())
        .with_root_context(scripts.document())
        .with_root_context(environment);
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

/// Each segmented control's `aria-label`, and its buttons.
fn segments(page: &str) -> Vec<(String, Vec<Button>)> {
    const OPEN: &str = "class=\"ds-segmented\" data-size=\"regular\" role=\"group\" aria-label=\"";
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

    // Appearance, Motion (a Space's three levels) and Accent are quire's `SpaceEditor`'s own
    // rows and names; Provider marks is mailo's row under it.
    let groups = [
        (
            "Appearance",
            ["System", "Light", "Dark"].as_slice(),
            "System",
        ),
        (
            "Motion",
            ["Calm", "Standard", "Extra"].as_slice(),
            "Standard",
        ),
        (
            "Accent",
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

    // quire's eight presets, each named for itself (they replaced the six retired accents).
    let presets = buttons_in(&page, "ds-presets");
    let names: Vec<&str> = presets
        .iter()
        .map(|button| button.attr("aria-label"))
        .collect();
    assert_eq!(names, PRESET_NAMES, "{page}");
    assert_eq!(presets.len(), PRESETS.len());
    assert!(page.contains("Refresh icons"), "{page}");
    assert!(
        page.contains("Sidebar text on the colour"),
        "no readout: {page}"
    );
    assert_eq!(
        page.matches("class=\"ds-handle\" role=\"slider\"").count(),
        2,
        "the Work Space has two dots: {page}"
    );
    assert!(
        page.contains("class=\"ds-slider\" role=\"slider\" tabindex=\"0\" aria-label=\"Grain\""),
        "grain is not quire's Slider: {page}"
    );
}

#[tokio::test]
async fn escape_puts_the_space_back_exactly_and_writes_nothing() {
    let (mut dom, seen, built, _scripts) = opened();
    let file = built.dirs.config.join("spaces.json");
    let stored = std::fs::read(&file).unwrap_or_else(|e| panic!("{e}"));
    let saved = space::load(&built.dirs.config).current_space();

    let _ = type_into(&mut dom, seen.one("value", "Work"), "Elsewhere");
    let _ = click(
        &mut dom,
        seen.after("aria-label", "Appearance", "aria-pressed")[2],
    );
    let edited = dioxus_ssr::render(&dom);
    assert!(
        edited.contains("Edit the Elsewhere Space"),
        "the name did not reach the frame live: {edited}"
    );
    assert_eq!(
        root_attr(&edited, "data-theme").as_deref(),
        Some("dark"),
        "choosing Dark did not repaint the frame live"
    );

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
    // And the frame was repainted as the saved Space: System again, which on a desktop with
    // no preference is light, not the Dark tried.
    assert_eq!(
        root_attr(&page, "data-theme").as_deref(),
        Some("light"),
        "Esc did not repaint the saved Space"
    );
}

#[tokio::test]
async fn save_writes_the_space_and_it_reads_back_the_same() {
    let (mut dom, seen, built, _scripts) = opened();
    let before = space::load(&built.dirs.config).current_space();

    let _ = type_into(&mut dom, seen.one("value", "Work"), "Studio");
    let _ = click(
        &mut dom,
        seen.after("aria-label", "Appearance", "aria-pressed")[2],
    );
    let live = dioxus_ssr::render(&dom);
    let _ = click(
        &mut dom,
        seen.after("aria-label", "Accent", "aria-pressed")[1],
    );
    // Grain is quire's Slider: a key moves it one step, 35 to 80 in 45.
    let grain = seen.one("aria-label", "Grain");
    for _ in 35..80 {
        press(&mut dom, "ArrowRight", u32::try_from(grain.0).unwrap_or(0));
    }
    let _ = click(&mut dom, seen.one("title", "Save this Space and close"));

    let page = dioxus_ssr::render(&dom);
    assert!(
        !page.contains("aria-label=\"Space editor\""),
        "Save left the editor open"
    );
    let want = Space {
        name: "Studio".to_owned(),
        look: SpaceLook {
            theme: Theme::Dark,
            card_accent: CardAccent::Postmark,
            grain: Grain(80),
            ..before.look.clone()
        },
        ..before.clone()
    };
    assert_ne!(before, want, "the edits above changed nothing");
    assert_eq!(space::load(&built.dirs.config).current_space(), want);
    assert_eq!(want.motion, Motion::Standard);
    assert_eq!(
        root_attr(&live, "data-theme").as_deref(),
        Some("dark"),
        "choosing Dark did not repaint the frame live"
    );
}

#[tokio::test]
#[ignore]
async fn render_the_space_editor_to_a_file() {
    // Rendered once per scheme, so the swatches and the field are the ones each scheme shows.
    for (suffix, scheme) in [("", ds::Scheme::Light), ("-dark", ds::Scheme::Dark)] {
        let (dom, _, _built, _) = opened_with(crate::ui::fixtures::in_scheme(scheme));
        crate::ui::fixtures::write_page(
            &format!("space-editor{suffix}"),
            &crate::ui::fixtures::page(&dioxus_ssr::render(&dom), ""),
        );
    }
}
