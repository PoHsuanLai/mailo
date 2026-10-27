//! The search in the list's toolbar: a small affordance, not a field. A magnifier while the list
//! is not searched; while it is, the search itself across the whole header, with a way to keep it
//! as a view and a way to clear it, so the person sees the list is narrowed after the panel has
//! gone. A press on either brings up the panel ([`super::spotlight`]), as ⌘K does; nothing is
//! typed here.

use super::spotlight::summon;
use crate::ui::copy;
use crate::ui::press::on_primary;
use crate::ui::view::{Bar, Shell};
use chrono::Utc;
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::prelude::*;
use ds::style::icon::render::IconPx;
use ds::style::tokens::control_size::ControlSize;
use mail_core::search;

/// What the toolbar's search is called, to a screen reader and to a test.
pub(in crate::ui) const BOX_LABEL: &str = "Search";

/// What the button that clears the search is called.
pub(in crate::ui) const CLEAR_LABEL: &str = copy::aria::CLEAR_SEARCH;

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

/// A search as the header shows it: its operators as the panel's chips, in the order typed, and
/// the rest of the line as words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Terms {
    pub chips: Vec<String>,
    pub words: String,
}

/// `query` split as the header shows it. The operators are the ones the panel draws as chips
/// under its field (`items::chips` once a search is typed); each is taken out of the line once,
/// and what is left keeps its order, its spaces closed up.
pub(in crate::ui) fn terms(query: &str) -> Terms {
    let chips = search::parse(query, &Utc, &|_| Vec::new()).operator_tokens;
    let mut rest = query.to_owned();
    for chip in &chips {
        if let Some(at) = rest.find(chip.as_str()) {
            rest.replace_range(at..at + chip.len(), " ");
        }
    }
    let words = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    Terms { chips, words }
}

/// The toolbar's magnifier: a press brings up the panel.
#[component]
pub(in crate::ui) fn SearchBox(shell: Signal<Shell>) -> Element {
    let keys = crate::ui::actions::tip_own(crate::ui::actions::Own::Search);
    rsx! {
        div { class: "bar",
            Button {
                bezel: Bezel::Toolbar,
                size: ControlSize::Large,
                image: ImagePosition::Only,
                label: BOX_LABEL,
                icon: Some(IconSource::Glyph(Icon::Search)),
                title: Some(BOX_LABEL.to_owned()),
                title_shortcut: keys,
                onclick: on_primary(move || summon(shell)),
            }
        }
    }
}

/// The list's header while a search narrows the list: the search, as the panel's field shows it
/// but at the toolbar's size, across the header, with where it looks at its end (`scope`, in the
/// danger ink when the search cannot be run); then Save as View and the × that clears it. A
/// press on the search brings the panel back over it.
#[component]
pub(in crate::ui) fn SearchHead(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    /// Where the search looks, or why it cannot be run.
    scope: String,
    /// Whether `scope` says the search cannot be run.
    bad: bool,
) -> Element {
    let Terms { chips, words } = terms(&shell.read().search);
    rsx! {
        div {
            class: "search-face",
            role: "button",
            tabindex: "0",
            "aria-label": copy::aria::EDIT_SEARCH,
            onclick: move |_| summon(shell),
            onkeydown: move |event: KeyboardEvent| {
                let key = event.key().to_string();
                if event.modifiers().is_empty() && matches!(key.as_str(), " " | "Enter") {
                    event.prevent_default();
                    event.stop_propagation();
                    summon(shell);
                }
            },
            span { class: "search-glyph",
                IconView { source: IconSource::Glyph(Icon::Search), size: IconSize::Px(IconPx(16)) }
            }
            span { class: "search-terms",
                for chip in chips {
                    Chip { variant: ChipVariant::Token, text: chip }
                }
                if !words.is_empty() {
                    span { class: "search-words", "{words}" }
                }
            }
            span { class: if bad { "search-scope bad" } else { "search-scope" }, "{scope}" }
        }
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            label: copy::aria::SAVE_AS_VIEW,
            icon: Some(IconSource::Glyph(Icon::Plus)),
            title: Some(copy::tooltips::SAVE_AS_VIEW.to_owned()),
            onclick: on_primary(move || {
                let search = shell.peek().search.clone();
                crate::ui::views::open_new(shell, &search);
            }),
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
    }
}

/// The search emptied, from the header's × or Esc in the list: the whole place listed again.
pub(in crate::ui) fn clear(mut shell: Signal<Shell>, mut pages: Signal<u32>) {
    shell.write().search = String::new();
    pages.set(1);
}

#[cfg(test)]
mod tests {
    use super::{Affordance, Terms, affordance, terms};
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

    #[test]
    fn the_header_draws_the_operators_as_chips_and_the_rest_as_words() {
        // A search, its chips and its words.
        type Case<'a> = (&'a str, &'a [&'a str], &'a str);
        let cases: &[Case] = &[
            ("lunch", &[], "lunch"),
            ("from:ada lunch", &["from:ada"], "lunch"),
            (
                "lunch  from:ada  on friday",
                &["from:ada"],
                "lunch on friday",
            ),
            (
                "is:unread subject:\"lunch plans\" ada",
                &["is:unread", "subject:\"lunch plans\""],
                "ada",
            ),
            ("from:ada", &["from:ada"], ""),
        ];
        for (query, chips, words) in cases {
            let want = Terms {
                chips: chips.iter().map(|chip| (*chip).to_owned()).collect(),
                words: (*words).to_owned(),
            };
            assert_eq!(terms(query), want, "{query}");
        }
    }
}
