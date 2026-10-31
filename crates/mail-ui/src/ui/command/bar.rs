//! The search in the list's toolbar: a small affordance, not a field. A magnifier while the list
//! is not searched; while it is, the search itself across the whole header, with a way to keep it
//! as a view and a way to clear it, so the person sees the list is narrowed after the panel has
//! gone. A press on either brings up the panel ([`super::spotlight`]), as ⌘K does; nothing is
//! typed here.

use super::spotlight::summon;
use crate::ui::press::on_primary;
use crate::ui::view::{Bar, Shell};
use chrono::Utc;
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::prelude::*;
use ds::style::icon::render::IconPx;
use ds::style::tokens::control_size::ControlSize;
use mail_client::search;
use mail_ui_model::copy;

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
/// under its field (`items::chips` once a search is typed). Each is cut out of the line at the
/// place the parser read it from, token by token, so a word that merely contains an operator's
/// text (`xfrom:ada` before `from:ada`) stays a word; what is left keeps its order, its spaces
/// closed up.
pub(in crate::ui) fn terms(query: &str) -> Terms {
    let chips = search::parse(query, &Utc, &mail_client::query::unnamed).operator_tokens;
    let spans = token_spans(query);
    let mut taken = vec![false; spans.len()];
    let mut at = 0;
    for chip in &chips {
        // An either-or chip is its terms joined by ` OR `, over the joiners as typed.
        let parts: Vec<&str> = chip.split(" OR ").collect();
        while at < spans.len() {
            match chip_run(query, &spans, at, &parts) {
                Some(last) => {
                    taken[at..=last].fill(true);
                    at = last + 1;
                    break;
                }
                None => at += 1,
            }
        }
    }
    let words = spans
        .iter()
        .zip(&taken)
        .filter(|(_, taken)| !**taken)
        .map(|((from, to), _)| &query[*from..*to])
        .collect::<Vec<_>>()
        .join(" ");
    Terms { chips, words }
}

/// The index of the last token of the operator that starts at token `at` and reads as `parts`
/// (one term, or several joined by `OR` or `|`), if one does.
fn chip_run(query: &str, spans: &[(usize, usize)], at: usize, parts: &[&str]) -> Option<usize> {
    let text = |index: usize| spans.get(index).map(|(from, to)| &query[*from..*to]);
    let mut last = at;
    for (n, part) in parts.iter().enumerate() {
        if n > 0 {
            if !matches!(text(last + 1), Some("OR" | "|")) {
                return None;
            }
            last += 1;
        }
        let next = if n == 0 { at } else { last + 1 };
        if text(next) != Some(*part) {
            return None;
        }
        last = next;
    }
    Some(last)
}

/// Move `to` past the next closing quote (or the end), taking its characters from `chars`.
fn close_quote(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, to: &mut usize) {
    for (at, quote) in chars.by_ref() {
        *to = at + quote.len_utf8();
        if quote == '"' {
            break;
        }
    }
}

/// Where each whitespace-separated token of `query` lies, as byte ranges. A quoted run, also as
/// a field's value (`subject:"lunch plans"`), is one token: the parser's own split.
fn token_spans(query: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut chars = query.char_indices().peekable();
    while let Some((from, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut to = from + c.len_utf8();
        if c == '"' {
            close_quote(&mut chars, &mut to);
        } else {
            while let Some(&(at, next)) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                chars.next();
                to = at + next.len_utf8();
                if next == '"' && query[from..to].ends_with(":\"") {
                    close_quote(&mut chars, &mut to);
                    break;
                }
            }
        }
        out.push((from, to));
    }
    out
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
        // A search is filtered as a place is, and the row it shows is the same.
        crate::ui::list::quick::FilterButton { shell, pages }
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
    use super::{Affordance, SearchHead, Terms, affordance, terms};
    use crate::ui::view::{Bar, BarOpen, Shell};
    use dioxus::prelude::*;

    #[component]
    fn Searching() -> Element {
        let shell = use_signal(|| Shell {
            search: "lunch".to_owned(),
            ..Shell::default()
        });
        let pages = use_signal(|| 1_u32);
        rsx! { SearchHead { shell, pages, scope: "Inbox".to_owned(), bad: false } }
    }

    /// A search shown is the list's header, and the Filter button stays in it.
    #[test]
    fn the_search_header_keeps_the_filter_button() {
        let mut dom = VirtualDom::new(Searching);
        dom.rebuild_in_place();
        let page = dioxus_ssr::render(&dom);
        assert!(page.contains(mail_ui_model::copy::aria::FILTER), "{page}");
    }

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
            // A word that holds an operator's text is a word; the operator is the token itself.
            ("xfrom:ada from:ada", &["from:ada"], "xfrom:ada"),
            (
                "from:ada OR from:bob lunch",
                &["from:ada OR from:bob"],
                "lunch",
            ),
            ("a from:ada | from:bob b", &["from:ada OR from:bob"], "a b"),
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
