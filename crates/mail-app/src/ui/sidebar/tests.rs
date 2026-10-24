use super::super::app::App;
use crate::ui::fixtures::empty;
use crate::ui::space::{Mail, built};
use dioxus::prelude::*;
use ds::prelude::SpaceLook;

#[tokio::test]
async fn the_foot_dots_are_buttons_that_say_which_space_is_on() {
    let (store, _dir) = empty();
    let spaces = built(
        ["Work", "Home", "Club"]
            .into_iter()
            .map(|name| (name.to_owned(), SpaceLook::default(), Mail::default()))
            .collect(),
        1,
    );
    let mut dom = VirtualDom::new(App)
        .with_root_context(store)
        .with_root_context(spaces);
    dom.rebuild_in_place();
    let page = dioxus_ssr::render(&dom);
    let dots = buttons_in(&page, "ds-spaces-dots");
    let labels: Vec<&str> = dots
        .iter()
        .map(|button| button.attr("aria-label"))
        .collect();
    assert_eq!(labels, ["Work Space", "Home Space", "Club Space"], "{page}");
    assert_eq!(
        pressed(&dots, |button| button.attr("aria-label")),
        ["Home Space"],
        "{page}"
    );
    // Each dot's tip, which names its key, is quire's hover tip: `tests/app/native_harness.rs`
    // rests the pointer on one and reads it.
    assert!(
        !page.contains("class=\"appearance"),
        "the gear popover is still on the frame: {page}"
    );
}

#[tokio::test]
async fn the_foot_holds_the_dots_and_one_menu_button_and_no_settings_or_sidebar_buttons() {
    let (store, _dir) = empty();
    let spaces = built(
        ["Work", "Home", "Club"]
            .into_iter()
            .map(|name| (name.to_owned(), SpaceLook::default(), Mail::default()))
            .collect(),
        0,
    );
    let mut dom = VirtualDom::new(App)
        .with_root_context(store)
        .with_root_context(spaces);
    dom.rebuild_in_place();
    let page = dioxus_ssr::render(&dom);
    let buttons = buttons_in(&page, "ds-spaces-foot");
    let labels: Vec<&str> = buttons
        .iter()
        .map(|button| button.attr("aria-label"))
        .collect();
    // The dots and the one chevron at the right, and nothing else: Downloads is in the title
    // row now. No `+`: the menu's New Space replaces it.
    assert_eq!(
        labels,
        [
            "Work Space",
            "Home Space",
            "Club Space",
            crate::ui::copy::aria::SIDEBAR_MENU
        ],
        "{page}"
    );
    assert_eq!(
        labels.last().copied(),
        Some(crate::ui::copy::aria::SIDEBAR_MENU),
        "{page}"
    );
    for gone in [
        crate::ui::copy::titles::SETTINGS,
        crate::ui::copy::aria::SIDEBAR,
        crate::ui::copy::menus::HIDE_SIDEBAR,
        crate::ui::copy::menus::SHOW_SIDEBAR,
    ] {
        assert!(
            !labels.contains(&gone),
            "{gone} is still on the foot: {labels:?}"
        );
    }
}

/// The Space's name heads the sidebar with one Space or several; the foot's dots come only with a
/// second Space, when there are Spaces to tell apart.
#[tokio::test]
async fn the_space_s_name_heads_the_sidebar_and_a_second_space_brings_the_dots() {
    let page = |names: &[&str]| {
        let (store, _dir) = empty();
        let spaces = built(
            names
                .iter()
                .map(|name| ((*name).to_owned(), SpaceLook::default(), Mail::default()))
                .collect(),
            0,
        );
        let mut dom = VirtualDom::new(App)
            .with_root_context(store)
            .with_root_context(spaces);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    };
    let one = page(&["Work"]);
    assert!(one.contains("The Work Space"), "{one}");
    assert!(!one.contains("class=\"ds-spaces-dots\""), "{one}");
    let two = page(&["Work", "Home"]);
    assert!(two.contains("The Work Space"), "{two}");
    assert!(two.contains("class=\"ds-spaces-dots\""), "{two}");
}

/// With an account that has a server, Sync is the foot's mark, at its start; the list's header
/// is its title, New Message, Search and the ⋯ that holds Group, Row Properties and Sync Now.
#[tokio::test]
async fn sync_is_the_foots_mark_and_the_list_header_is_its_title_and_three_icons() {
    let (store, _dir) = crate::ui::fixtures::seeded();
    let page = crate::ui::fixtures::markup(store);
    let labels = |class: &str| -> Vec<String> {
        buttons_in(&page, class)
            .iter()
            .map(|button| button.attr("aria-label").to_owned())
            .collect()
    };
    let foot = labels("ds-spaces-foot");
    assert_eq!(
        foot.first().map(String::as_str),
        Some(crate::ui::copy::aria::SYNC_NOW),
        "{foot:?}"
    );
    assert_eq!(
        labels("list-head"),
        [
            crate::ui::copy::aria::NEW_MESSAGE,
            crate::ui::command::BOX_LABEL,
            crate::ui::copy::aria::MORE,
        ],
        "{page}"
    );
}

pub(in crate::ui) struct Button {
    pub(in crate::ui) attrs: Vec<(String, String)>,
}

impl Button {
    pub(in crate::ui) fn attr(&self, name: &str) -> &str {
        self.attrs
            .iter()
            .find(|(got, _)| got == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or("")
    }
}

/// The identity of every button whose `aria-pressed` is `true`.
/// Every button in the group must say true or false, so a missing attribute fails here
/// rather than looking like "nothing is pressed".
pub(in crate::ui) fn pressed<'a>(
    buttons: &'a [Button],
    id: impl Fn(&'a Button) -> &'a str,
) -> Vec<&'a str> {
    let mut on = Vec::new();
    for button in buttons {
        let state = button.attr("aria-pressed");
        assert!(
            state == "true" || state == "false",
            "aria-pressed is {state:?} on {}",
            id(button)
        );
        if state == "true" {
            on.push(id(button));
        }
    }
    on
}

/// The `<button>` elements inside the element whose class list contains `class`.
pub(in crate::ui) fn buttons_in(html: &str, class: &str) -> Vec<Button> {
    let mut buttons = Vec::new();
    let mut group_depth: Option<usize> = None;
    let mut depth = 0usize;
    let mut index = 0usize;
    let bytes = html.as_bytes();
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let rest = &html[index..];
        if rest.starts_with("</") {
            let Some(end) = rest.find('>') else { break };
            if group_depth == Some(depth) {
                group_depth = None;
            }
            depth = depth.saturating_sub(1);
            index += end + 1;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            let Some(end) = rest.find('>') else { break };
            index += end + 1;
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let raw = &rest[1..end];
        let self_closing = raw.ends_with('/');
        let raw = raw.trim_end_matches('/').trim();
        let name = raw.split_whitespace().next().unwrap_or("");
        let attrs = attributes(raw);
        let in_group = group_depth.is_some();
        if !self_closing {
            depth += 1;
            if group_depth.is_none() && has_class(&attrs, class) {
                group_depth = Some(depth);
            }
        }
        if in_group && name == "button" {
            buttons.push(Button { attrs });
        }
        index += end + 1;
    }
    buttons
}

fn has_class(attrs: &[(String, String)], class: &str) -> bool {
    attrs.iter().any(|(name, value)| {
        name == "class" && value.split_whitespace().any(|token| token == class)
    })
}

fn attributes(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = match raw.find(char::is_whitespace) {
        Some(space) => raw[space..].trim_start(),
        None => return out,
    };
    while !rest.is_empty() {
        let name_end = rest.find(['=', ' ', '\n', '\t']).unwrap_or(rest.len());
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        if name.is_empty() {
            continue;
        }
        if !rest.starts_with('=') {
            out.push((name.to_string(), String::new()));
            continue;
        }
        rest = rest[1..].trim_start();
        if rest.is_empty() {
            out.push((name.to_string(), String::new()));
            break;
        }
        let quote = rest.as_bytes()[0];
        if quote == b'"' || quote == b'\'' {
            rest = &rest[1..];
            let value_end = rest.find(quote as char).unwrap_or(rest.len());
            out.push((name.to_string(), rest[..value_end].to_string()));
            rest = rest.get(value_end + 1..).unwrap_or("").trim_start();
        } else {
            let value_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push((name.to_string(), rest[..value_end].to_string()));
            rest = rest[value_end..].trim_start();
        }
    }
    out
}
