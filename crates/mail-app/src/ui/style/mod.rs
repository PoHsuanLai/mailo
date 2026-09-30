//! The shell's stylesheet: layout only.
//!
//! Real `.css` files, stitched at compile time and drawn by `ds::prelude::AppStyle`, which puts
//! them in quire's `app` cascade layer (after `ds`, before the person's own `style.css`). What
//! these files hold is where mailo puts things (the window's grid, the reader's column, the
//! composer's page); how anything looks is quire's: every colour, size, radius, weight, shadow
//! and duration is a token, every control a quire component, every animation quire's. quire's
//! lint holds the line (`lint::assert_clean` at `Profile::Strict`, and the markup lint over the
//! rendered window); a rule that needs a value quire has no token for is a request to quire
//! (FINDINGS "quire requests"), never a literal here.
//!
//! The order below is the cascade and is load-bearing. It is one `concat!` and not several
//! named pieces because `concat!` takes literals only: `include_str!` expands to one, a `const`
//! does not.

#[cfg(test)]
mod exceptions;

pub(super) const STYLE: &str = concat!(
    include_str!("shell.css"),
    include_str!("editor.css"),
    include_str!("list.css"),
    include_str!("menus.css"),
    include_str!("reader.css"),
    include_str!("invite.css"),
    include_str!("hover.css"),
    include_str!("composer.css"),
    include_str!("contacts.css"),
    include_str!("files.css"),
    include_str!("accounts.css"),
    include_str!("rules.css"),
    include_str!("pgp.css"),
    include_str!("controls.css"),
);

#[cfg(test)]
pub(in crate::ui) mod tests {
    use super::STYLE;
    use ds_lint::{LintConfig, Offence, Profile, Rule, assert_clean, markup};

    /// Everything the window's markup is styled by: quire's stylesheet, then mailo's.
    pub(in crate::ui) fn full_css() -> String {
        format!("{}\n{STYLE}", ds::stylesheet())
    }

    /// The custom properties mailo's own rules declare on an element, each for its own box.
    const PER_ELEMENT: &[&str] = &[];

    fn config() -> LintConfig {
        LintConfig {
            profile: Profile::Strict,
            own_vars: PER_ELEMENT.iter().map(|name| (*name).to_owned()).collect(),
            exceptions: super::exceptions::STYLE,
            ..LintConfig::new(&ds::kits())
        }
    }

    /// The offences the markup lint finds in `html` drawn with mailo's and quire's sheets: raw
    /// controls, raw vectors and literal colours in a `style`, beside unstyled classes.
    pub(in crate::ui) fn markup_offences(html: &str) -> Vec<Offence> {
        markup(
            html,
            &full_css(),
            &LintConfig {
                exceptions: super::exceptions::MARKUP,
                ..LintConfig::new(&ds::kits())
            },
        )
    }

    /// The window's first frame with a conversation open in the reader, the command menu's
    /// open menus, and the Space editor, which the first frame never shows: opened here
    /// through the Space's name as a person would.
    async fn frame_markup() -> String {
        use crate::ui::app::App;
        use crate::ui::fixtures::work;
        use dioxus::prelude::*;
        let built = work();
        let store = built.store.clone();
        let mut dom = VirtualDom::new(App)
            .with_root_context(built.store)
            .with_root_context(built.dirs);
        dom.rebuild_in_place();
        let mut menus =
            VirtualDom::new(crate::ui::command::tests::OpenMenus).with_root_context(store);
        menus.rebuild_in_place();
        // The palette floats in the root's overlay, drawn the render after it asks.
        crate::ui::fixtures::drain(&mut menus);
        let editor = crate::ui::space_editor::tests::editor_open_markup();
        assert!(
            editor.contains("aria-label=\"Space editor\""),
            "the editor did not open: {editor}"
        );
        dioxus_ssr::render(&dom) + &dioxus_ssr::render(&menus) + &editor
    }

    /// Coherence rule 1: mailo's own stylesheet at quire's strictest profile, spacing included.
    /// Every exception is named with its reason, and one that no longer suppresses anything
    /// fails here (`assert_clean` reports a stale exception) rather than hiding a later offence.
    #[test]
    fn our_stylesheet_lints_clean() {
        assert_clean(STYLE, &config());
    }

    /// Coherence rule 2: no class nothing styles, no raw control, no raw vector, no literal
    /// colour in a `style`, except those `exceptions::MARKUP` names with their reasons.
    #[tokio::test]
    async fn the_frame_draws_no_raw_markup_and_no_unstyled_class() {
        let page = frame_markup().await;
        let offences = markup_offences(&page);
        assert!(offences.is_empty(), "{offences:#?}");
    }

    /// The failure is the class name: a stylesheet that merely exists would pass a test that
    /// only asked "is there CSS".
    #[test]
    fn a_class_with_no_rule_is_named() {
        let offences = markup_offences("<div class=\"zz-missing\"></div>");
        assert!(
            offences
                .iter()
                .any(|offence| offence.rule == Rule::UnstyledClass
                    && offence.text.contains("zz-missing")),
            "{offences:#?}"
        );
    }

    /// The declarations of the rule whose selector is exactly `selector`, comments removed.
    fn rule_body(selector: &str) -> String {
        let mut css = String::new();
        let mut rest = STYLE;
        while let Some(start) = rest.find("/*") {
            css.push_str(&rest[..start]);
            rest = rest[start + 2..]
                .split_once("*/")
                .map_or("", |(_, after)| after);
        }
        css.push_str(rest);
        css.split('}')
            .filter_map(|rule| rule.split_once('{'))
            .find(|(prelude, _)| prelude.trim() == selector)
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default()
    }

    /// The window is one grid whose text track can shrink; without `minmax(0, 1fr)` a long
    /// subject widens the window's column past the screen.
    #[test]
    fn the_window_grid_can_shrink() {
        let app = rule_body(".app");
        assert!(
            app.contains("minmax(0, 1fr)"),
            ".app has no minmax(0, 1fr) track: {app}"
        );
    }

    /// The card is paper, not frame: it resets the text colour to the paper's ink, or anything on
    /// it would inherit the frame's ink, which reads in one scheme and vanishes in the other.
    #[test]
    fn the_paper_sets_its_own_ink() {
        let card = rule_body(".card");
        assert!(
            card.contains("color: var(--ink)"),
            ".card must reset the text colour to the paper's ink: {card}"
        );
    }
}
