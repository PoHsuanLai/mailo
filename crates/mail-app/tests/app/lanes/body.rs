//! The composer's body as a person reads it, for the formatting lanes: each block's element and
//! its text, top to bottom.

use ds_harness::{Harness, Query};

use super::drive::Key;
use super::window::Window;

/// The elements a body block is drawn as.
const BLOCKS: [&str; 9] = [
    "p",
    "h2",
    "h3",
    "h4",
    "blockquote",
    "pre",
    "ol",
    "ul",
    "div.obj",
];

/// The body, a block a line: `h2 Plan`, `ul [a | b]`, `ul.todo [call]`, `p Steps`. An empty
/// paragraph is `p`.
pub fn outline(harness: &Harness) -> Vec<String> {
    let mut out = Vec::new();
    for n in 1..=harness.count(".c-body > *") {
        let at = format!(".c-body > :nth-child({n})");
        let Some(tag) = BLOCKS
            .iter()
            .find(|tag| harness.count(&format!(".c-body > {tag}:nth-child({n})")) == 1)
        else {
            continue;
        };
        let line = match *tag {
            "ul" | "ol" => {
                let todo = harness.count(&format!("{at}.todo")) == 1;
                let items: Vec<String> = (1..=harness.count(&format!("{at} > li")))
                    .map(|i| text(harness, &format!("{at} > li:nth-child({i})")))
                    .collect();
                let name = if todo { "ul.todo" } else { tag };
                format!("{name} [{}]", items.join(" | "))
            }
            tag => {
                let said = text(harness, &at);
                if said.is_empty() {
                    tag.to_owned()
                } else {
                    format!("{tag} {said}")
                }
            }
        };
        out.push(line);
    }
    out
}

/// `selector`'s text, its trailing line breaks dropped.
fn text(harness: &Harness, selector: &str) -> String {
    harness
        .text_of(selector)
        .unwrap_or_default()
        .trim_end_matches('\n')
        .to_owned()
}

/// A new message, the keyboard in its body.
pub fn composing() -> Window {
    let mut window = Window::open(|_| {});
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer", |h| h.count(".cpage .c-body") == 1);
    window.click(".c-body");
    window.until("the body has the keyboard", |h| {
        h.focus_of(".c-body") == ds_harness::FocusState::Focused
    });
    window
}

/// Select the `n` letters before the caret, as Shift+Left does.
pub fn select_back(window: &mut Window, n: usize) {
    window.press(&[Key::Shift], Key::Left, n);
}

/// Whether the body's run `selector` reads `text`.
pub fn run_is(harness: &Harness, selector: &str, text: &str) -> bool {
    harness
        .text_of(&format!(".c-body {selector}"))
        .is_some_and(|run| run == text)
}
