//! Delete Space in the editor: the button's rule, the sheet that asks, and what confirming does.
//!
//! A Space is a look, some pins and an account scope; deleting one never touches mail. The sheet
//! says so, names the Space, and only its button deletes. Confirming puts the draft back first
//! (so nothing half-edited is kept), removes the Space, renumbers Today to match, shows the Space
//! that is now current as a switch would, and writes `spaces.json`.

use super::cancel;
use crate::ui::appearance::WindowDirs;
use crate::ui::frame::keep;
use crate::ui::press::{SheetClose, available, on_primary};
use crate::ui::space::Spaces;
use crate::ui::space::edit::Draft;
use crate::ui::space::remove::{Showing, remove};
use crate::ui::switch::restore;
use crate::ui::today::{self, Today};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;

/// What the sheet says, for the Space it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Words {
    /// The sheet's heading and accessible name.
    pub title: String,
    /// What goes and what stays.
    pub body: String,
    /// The confirming button.
    pub confirm: String,
}

/// The sheet's words for the Space called `name`. A Space with no name is "this Space".
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

/// Whether Delete Space can be pressed with `count` Spaces: not for the last one.
pub(super) fn offered(count: usize) -> bool {
    count > 1
}

/// Ask about the Space the editor is on. Nothing opens for the last Space.
pub(super) fn ask(
    mut shell: Signal<Shell>,
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
) {
    let Some(index) = editing.read().as_ref().map(|draft| draft.index) else {
        return;
    };
    if offered(spaces.read().spaces.len()) {
        shell.write().removing_space = Some(index);
    }
}

/// Close the sheet, deleting nothing, and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().removing_space = None;
    crate::ui::host::Host::focus_app();
}

/// The sheet's own button: put the draft back, delete the Space, close the editor.
fn confirm(
    mut shell: Signal<Shell>,
    mut spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    pages: Signal<u32>,
    mut today_list: Signal<Today>,
) {
    let Some(index) = shell.peek().removing_space else {
        return;
    };
    shell.write().removing_space = None;
    cancel(editing, spaces);
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
}

/// The confirmation, over the editor while it is open.
#[component]
pub(super) fn RemoveSheet(
    shell: Signal<Shell>,
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    pages: Signal<u32>,
    today: Signal<Today>,
) -> Element {
    let Some(index) = shell.read().removing_space else {
        return rsx! {};
    };
    let name = spaces
        .read()
        .spaces
        .get(index)
        .map(|space| space.name.clone())
        .unwrap_or_default();
    let said = words(&name);
    rsx! {
        div {
            class: "rules-wrap",
            onclick: move |_| close(shell),
            div {
                class: "rules destroy-sheet",
                role: "alertdialog",
                aria_label: "{said.title}",
                onclick: move |event| event.stop_propagation(),
                div { class: "rules-head",
                    h3 { "{said.title}" }
                    SheetClose { label: "Cancel", on_close: move |()| close(shell) }
                }
                div { class: "rules-part",
                    p { class: "destroy-body", "{said.body}" }
                    div { class: "rules-acts",
                        Button {
                            size: ControlSize::Small,
                            label: said.confirm.clone(),
                            icon: Icon::Trash,
                            availability: available(offered(spaces.read().spaces.len())),
                            common: Common {
                                aria_label: Some(said.confirm.clone()),
                                ..Common::default()
                            },
                            onclick: on_primary(move || confirm(shell, spaces, editing, pages, today)),
                        }
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
