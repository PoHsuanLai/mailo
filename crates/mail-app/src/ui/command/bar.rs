//! The search in the list's toolbar: a small affordance, not a field. A magnifier while the list
//! is not searched; the search itself, with a way to clear it, while it is, so the person sees
//! the list is narrowed after the panel has gone. A press on either brings up the panel
//! ([`super::spotlight`]), as ⌘K does; nothing is typed here.

use super::spotlight::summon;
use crate::ui::press::on_primary;
use crate::ui::view::{Bar, Shell};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;

/// What the toolbar's search is called, to a screen reader and to a test.
pub(in crate::ui) const BOX_LABEL: &str = "Search";

/// What the button that clears the search is called.
pub(in crate::ui) const CLEAR_LABEL: &str = "Clear search";

/// What the toolbar shows for the search: the magnifier alone, or the search being shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Affordance {
    /// Nothing searched: the magnifier.
    Idle,
    /// The list is narrowed by this search, and the panel is not up.
    Searching(String),
}

/// The affordance for the window's search and whether the panel is up. While the panel is up,
/// the panel shows what is typed; the toolbar keeps its magnifier.
pub(in crate::ui) fn affordance(search: &str, bar: &Bar) -> Affordance {
    match bar {
        Bar::Closed if !search.trim().is_empty() => Affordance::Searching(search.to_owned()),
        Bar::Closed | Bar::Open(_) => Affordance::Idle,
    }
}

/// The toolbar's search: a press brings up the panel; the × beside a search empties it.
#[component]
pub(in crate::ui) fn SearchBox(shell: Signal<Shell>, pages: Signal<u32>) -> Element {
    let shown = affordance(&shell.read().search, &shell.read().bar);
    let title = "Search".to_owned();
    let keys = crate::ui::actions::tip_own(crate::ui::actions::Own::Search);
    rsx! {
        div { class: "bar",
            match shown {
                Affordance::Idle => rsx! {
                    Button {
                        bezel: Bezel::Toolbar,
                        size: ControlSize::Large,
                        image: ImagePosition::Only,
                        label: BOX_LABEL,
                        icon: Some(IconSource::Glyph(Icon::Search)),
                        title: Some(title),
                        title_shortcut: keys,
                        onclick: on_primary(move || summon(shell)),
                    }
                },
                Affordance::Searching(search) => rsx! {
                    Button {
                        bezel: Bezel::Toolbar,
                        size: ControlSize::Large,
                        image: ImagePosition::Leading,
                        label: search,
                        icon: Some(IconSource::Glyph(Icon::Search)),
                        title: Some(title),
                        title_shortcut: keys,
                        common: crate::ui::common::classed("search-shown"),
                        onclick: on_primary(move || summon(shell)),
                    }
                    Button {
                        bezel: Bezel::Toolbar,
                        size: ControlSize::Large,
                        image: ImagePosition::Only,
                        label: CLEAR_LABEL,
                        icon: Some(IconSource::Glyph(Icon::X)),
                        title: Some(CLEAR_LABEL.to_owned()),
                        onclick: on_primary(move || clear(shell, pages)),
                    }
                },
            }
        }
    }
}

/// The search emptied from the toolbar: the whole place listed again.
fn clear(mut shell: Signal<Shell>, mut pages: Signal<u32>) {
    shell.write().search = String::new();
    pages.set(1);
}

#[cfg(test)]
mod tests {
    use super::{Affordance, affordance};
    use crate::ui::view::{Bar, BarOpen};

    #[test]
    fn the_toolbar_shows_the_search_only_while_the_panel_is_down() {
        let open = Bar::Open(BarOpen::over(String::new()));
        let cases: &[(&str, &str, &Bar, Affordance)] = &[
            ("nothing searched", "", &Bar::Closed, Affordance::Idle),
            ("only spaces", "  ", &Bar::Closed, Affordance::Idle),
            (
                "a search, panel down",
                "lunch",
                &Bar::Closed,
                Affordance::Searching("lunch".to_owned()),
            ),
            ("a search, panel up", "lunch", &open, Affordance::Idle),
        ];
        for (name, search, bar, want) in cases {
            assert_eq!(affordance(search, bar), *want, "{name}");
        }
    }
}
