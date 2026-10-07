//! Delete Space…, from the Space's menu: the popover that asks, where the menu stood, and what
//! confirming does.
//!
//! A Space is a look, some pins and an account scope; deleting one never touches mail. The
//! popover says so, names the Space, and only its button deletes; Escape or a click outside
//! deletes nothing. Confirming removes the Space, renumbers Today to match, shows the Space that
//! is now current as a switch would, and writes `spaces.json`.

use crate::ui::appearance::WindowDirs;
use crate::ui::frame::keep;
use crate::ui::press::on_primary;
use crate::ui::space::Spaces;
use crate::ui::space::edit::Draft;
use crate::ui::space::remove::{Showing, remove};
use crate::ui::switch::restore;
use crate::ui::today::{self, Today};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::overlays::popover::Arrow;
use ds::host::measure::Anchor;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;

/// What the popover says, for the Space it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Words {
    /// The popover's heading and accessible name.
    pub title: String,
    /// What goes and what stays.
    pub body: String,
    /// The confirming button.
    pub confirm: String,
}

/// The popover's words for the Space called `name`. A Space with no name is "this Space".
pub(super) fn words(name: &str) -> Words {
    let named = if name.trim().is_empty() {
        "this Space".to_owned()
    } else {
        format!("\u{201c}{}\u{201d}", name.trim())
    };
    Words {
        title: format!("Delete {named}?"),
        body: format!(
            "Your mail is not affected. Only {named}\u{2019}s look, pins and the accounts it \
             shows go. This cannot be undone."
        ),
        confirm: "Delete Space".to_owned(),
    }
}

/// Whether a Space can be deleted with `count` Spaces: not the last one.
pub(super) fn offered(count: usize) -> bool {
    count > 1
}

/// Leave the Space as it is and give the keyboard back to the window.
fn dismiss(mut shell: Signal<Shell>, mut editing: Signal<Option<Draft>>) {
    editing.set(None);
    shell.write().space_menu = None;
    crate::ui::host::Host::focus_app();
}

/// The popover's own button: delete Space `index`.
fn confirm(
    index: usize,
    mut shell: Signal<Shell>,
    mut spaces: Signal<Spaces>,
    mut editing: Signal<Option<Draft>>,
    pages: Signal<u32>,
    mut today_list: Signal<Today>,
) {
    editing.set(None);
    shell.write().space_menu = None;
    let removed = {
        let mut all = spaces.write();
        match remove(&mut all, index) {
            Ok(removed) => removed,
            Err(_) => return,
        }
    };
    today_list.write().drop_space(index);
    if let Some(dirs) = try_consume_context::<WindowDirs>() {
        let _ = today::save(&dirs.state, &today_list.read());
    }
    if removed.showing == Showing::Other {
        let (space, recall) = {
            let all = spaces.read();
            (
                all.current_space(),
                all.recall
                    .get(&removed.now_current)
                    .cloned()
                    .unwrap_or_default(),
            )
        };
        restore(&mut shell.write(), &space, &recall);
        let mut pages = pages;
        pages.set(1);
    }
    keep(&spaces.read());
    crate::ui::host::Host::focus_app();
}

/// The question, where the Space's menu stood. Nothing is offered for the last Space, whose menu
/// has no Delete row.
#[component]
pub(super) fn DeletePopover(
    at: Point,
    index: usize,
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
    pages: Signal<u32>,
    today: Signal<Today>,
) -> Element {
    if !offered(spaces.read().spaces.len()) {
        return rsx! {};
    }
    let name = spaces
        .read()
        .spaces
        .get(index)
        .map(|space| space.name.clone())
        .unwrap_or_default();
    let said = words(&name);
    rsx! {
        Popover {
            anchor: Anchor::Point(at),
            placement: Placement::new(Side::Bottom, Align::Start),
            gap: Px(2.0),
            arrow: Arrow::None,
            common: Common { aria_label: Some(said.title.clone()), ..Common::default() },
            onclose: move |()| dismiss(shell, editing),
            div { class: "space-part space-delete", role: "alertdialog",
                h3 { "{said.title}" }
                p { class: "destroy-body", "{said.body}" }
                div { class: "space-part-acts",
                    Button {
                        size: ControlSize::Large,
                        label: "Cancel",
                        onclick: on_primary(move || dismiss(shell, editing)),
                    }
                    Button {
                        size: ControlSize::Large,
                        label: said.confirm.clone(),
                        icon: Icon::Trash,
                        common: Common { aria_label: Some(said.confirm.clone()), ..Common::default() },
                        onclick: on_primary(move || confirm(index, shell, spaces, editing, pages, today)),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{offered, words};

    #[test]
    fn the_sheet_names_the_space_and_says_mail_is_safe() {
        // (name, title)
        const CASES: &[(&str, &str)] = &[
            ("Work", "Delete \u{201c}Work\u{201d}?"),
            ("  Home ", "Delete \u{201c}Home\u{201d}?"),
            ("", "Delete this Space?"),
            ("   ", "Delete this Space?"),
        ];
        for (name, title) in CASES {
            let said = words(name);
            assert_eq!(said.title, *title);
            assert!(
                said.body.contains("Your mail is not affected"),
                "{}",
                said.body
            );
        }
    }

    #[test]
    fn delete_is_offered_only_while_another_space_remains() {
        const CASES: &[(usize, bool)] = &[(0, false), (1, false), (2, true), (5, true)];
        for (count, expect) in CASES {
            assert_eq!(offered(*count), *expect, "{count} Spaces");
        }
    }
}
