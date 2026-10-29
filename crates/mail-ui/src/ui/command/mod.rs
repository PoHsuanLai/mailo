//! ⌘K, the one way in: a small affordance in the list's toolbar, and a panel at the top centre of
//! the window (Spotlight's) with one field. Empty, it lists Recent, the places and the verbs with
//! their keys; typed, it is the list's search, and the mail it finds fills the list while the
//! panel keeps the places, people and commands the text names.
//!
//! The rows are built in [`items`], regrouped into the panel's sections in [`sections`] and
//! drawn as quire's result rows by [`suggest`]; the toolbar's affordance is [`bar`], the panel
//! [`spotlight`], its keys [`keys`]; the commands are [`commands`], one type each; this file is
//! what a pick does.

mod bar;
mod commands;
mod items;
mod keys;
mod panel;
pub(in crate::ui) mod people;
mod recent;
mod sections;
mod spotlight;
mod suggest;
mod templates;

pub(in crate::ui) use bar::{Affordance, SearchBox, SearchHead, affordance, clear};
#[cfg(test)]
pub(in crate::ui) use bar::{BOX_LABEL, CLEAR_LABEL};
pub(in crate::ui) use commands::{Acts, Command, Settings, Sidebar, TidyUp};
pub(in crate::ui) use items::avatar_color;
#[cfg(test)]
pub(in crate::ui) use spotlight::{FIELD, LABEL};
pub(in crate::ui) use spotlight::{Spotlight, summon};

use crate::ui::view::{Bar, PageMenu, Shell};
use dioxus::prelude::*;
use items::Pick;
use sections::Choice;
use spotlight::Ctx;

/// The panel closed, and the keyboard back with the list.
fn close(mut shell: Signal<Shell>) {
    shell.write().bar = Bar::Closed;
    shell.write().page_menu = PageMenu::Closed;
    crate::ui::host::Host::focus_app();
}

/// The search the list showed before a command's name was typed over it, back in the list: the
/// name was never a search.
fn restore(mut shell: Signal<Shell>, mut pages: Signal<u32>) {
    let before = match &shell.peek().bar {
        Bar::Open(open) => open.before.clone(),
        Bar::Closed => return,
    };
    if shell.peek().search != before {
        shell.write().search = before;
        pages.set(1);
    }
}

/// Do what the picked row says. A recent conversation opens; a person's mail becomes the search;
/// a place, a Space or a command runs on the search there was.
fn act(ctx: Ctx, choice: Option<Choice>) {
    let Ctx {
        mut shell,
        mut pages,
        side_hidden,
        desk,
        ..
    } = ctx;
    let Some(choice) = choice else {
        return;
    };
    match choice {
        Choice::Pick(Pick::Open(id)) => {
            shell.write().open(id);
            close(shell);
        }
        Choice::Pick(Pick::From(email)) => {
            shell.write().search = format!("from:{email}");
            pages.set(1);
            close(shell);
        }
        Choice::Place(index) => {
            restore(shell, pages);
            crate::ui::sidebar::go_to(shell, pages, index);
            close(shell);
        }
        Choice::Draft(draft) => {
            close(shell);
            if let Some(desk) = desk {
                crate::ui::compose::reopen(desk, shell, draft);
            }
        }
        Choice::Space(index) => {
            restore(shell, pages);
            close(shell);
            if let Some(handle) = try_consume_context::<crate::ui::space::Handle>() {
                let _ = handle.switch_index(index);
            }
        }
        Choice::Pick(Pick::Action(command)) => perform(
            command,
            Acts {
                shell,
                pages,
                side_hidden,
            },
        ),
    }
}

/// Do what picking `command` does: the search there was comes back in the list, the panel
/// closes (unless the command keeps it or closes it at a moment of its own), and it runs. The
/// panel's rows and the sidebar's foot menu both come here.
pub(in crate::ui) fn perform(command: &dyn Command, acts: Acts) {
    restore(acts.shell, acts.pages);
    if command.closes_first() {
        close(acts.shell);
    }
    command.run(acts);
}

#[cfg(test)]
mod bar_tests;
#[cfg(test)]
pub(in crate::ui) mod pictures;
#[cfg(test)]
pub(in crate::ui) mod tests;
#[cfg(test)]
mod window_tests;
