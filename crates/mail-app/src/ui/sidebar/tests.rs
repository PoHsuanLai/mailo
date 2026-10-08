use super::super::app::App;
use crate::ui::fixtures::empty;
use crate::ui::space::{Mail, Scope, built};
use dioxus::prelude::*;
use ds::prelude::SpaceLook;

#[tokio::test]
async fn the_foot_dots_are_buttons_that_say_which_space_is_on() {
    let (store, _dir) = empty();
    let spaces = built(
        ["Work", "Home", "Club"]
            .into_iter()
            .map(|name| {
                (
                    name.to_owned(),
                    SpaceLook::default(),
                    Mail::over(Scope::All),
                )
            })
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
    assert!(
        dots.iter().enumerate().all(|(index, button)| {
            // The tip: quire writes it as the description under `Ds`.
            let tip = match button.attr("aria-description") {
                "" => button.attr("title"),
                tip => tip,
            };
            tip.ends_with(&format!("(⌘{})", index + 1))
        }),
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

mod tile_status {
    use super::super::panes::all_status;
    use ds::components::app::pin_tile::PinStatus;
    use ds::motion::detail::operation::Operation;

    fn attention(why: &str) -> PinStatus {
        PinStatus::Attention { why: why.into() }
    }

    #[test]
    fn the_all_tile_speaks_for_the_accounts_in_scope() {
        let busy = PinStatus::Busy(Operation::default());
        assert_eq!(all_status(&[]), PinStatus::Quiet);
        assert_eq!(
            all_status(&[PinStatus::Quiet, PinStatus::Quiet]),
            PinStatus::Quiet
        );
        assert_eq!(
            all_status(&[PinStatus::Quiet, busy.clone()]),
            busy,
            "work shows when nothing needs the person"
        );
        assert_eq!(all_status(&[busy.clone(), attention("a")]), attention("a"));
        assert_eq!(
            all_status(&[attention("a"), attention("b"), busy]),
            attention("2 accounts need attention.")
        );
    }
}
