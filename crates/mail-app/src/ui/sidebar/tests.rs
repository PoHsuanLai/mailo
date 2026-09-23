use super::super::app::App;
use crate::space::{Space, Spaces};
use crate::ui::fixtures::empty;
use dioxus::prelude::*;
use std::collections::BTreeMap;

fn named(name: &str) -> Space {
    Space {
        name: name.to_owned(),
        ..Space::default()
    }
}

#[tokio::test]
async fn the_foot_dots_are_buttons_that_say_which_space_is_on() {
    let (store, _dir) = empty();
    let spaces = Spaces {
        spaces: vec![named("Work"), named("Home"), named("Club")],
        current: 1,
        recall: BTreeMap::new(),
    };
    let mut dom = VirtualDom::new(App)
        .with_root_context(store)
        .with_root_context(spaces);
    dom.rebuild_in_place();
    let page = dioxus_ssr::render(&dom);
    let dots = buttons_in(&page, "space-dots");
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
    assert!(
        dots.iter().enumerate().all(|(index, button)| button
            .attr("title")
            .ends_with(&format!("(Ctrl {})", index + 1))),
        "a dot does not name its key: {page}"
    );
    assert!(page.contains("aria-label=\"New Space\""), "{page}");
    assert!(
        !page.contains("class=\"appearance"),
        "the gear popover is still on the frame: {page}"
    );
}

pub(in crate::ui) struct Button {
    pub(in crate::ui) attrs: Vec<(String, String)>,
    pub(in crate::ui) text: String,
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
            let text = if self_closing {
                String::new()
            } else {
                let after = index + end + 1;
                let close = html[after..].find("</button>").unwrap_or(0);
                html[after..after + close].trim().to_string()
            };
            buttons.push(Button { attrs, text });
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
