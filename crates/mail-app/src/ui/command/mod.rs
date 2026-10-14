//! The window's search: a small affordance in the list's toolbar, and a panel at the top centre
//! of the window (Spotlight's) with one field that searches the list, and under it the mail,
//! commands, places and people the text names. What the ⌘K menu offered, it offers, matched
//! and ranked as the menu did.
//!
//! The rows are built in [`items`], regrouped into the panel's sections in [`sections`] and
//! drawn as quire's result rows by [`suggest`]; the toolbar's affordance is [`bar`], the panel
//! [`spotlight`], its keys [`keys`]; this file is what a pick does.

mod bar;
mod items;
mod keys;
mod panel;
pub(in crate::ui) mod people;
mod recent;
mod sections;
mod spotlight;
mod suggest;
mod templates;

pub(in crate::ui) use bar::SearchBox;
#[cfg(test)]
pub(in crate::ui) use bar::{BOX_LABEL, CLEAR_LABEL};
pub(in crate::ui) use items::avatar_color;
#[cfg(test)]
pub(in crate::ui) use spotlight::{FIELD, LABEL};
pub(in crate::ui) use spotlight::{Spotlight, summon};

use super::ops::{Composes, compose_new, start_composing};
use crate::ui::view::{Bar, BarListing, PageMenu, Shell, Source};
use dioxus::prelude::*;
use items::{CommandId, Look, Pick};
use mail_core::SqliteStore;
use mail_core::place::place_filter;
use sections::Choice;
use spotlight::Ctx;
use std::sync::Arc;

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

/// Do what the picked row says. A mail opens with the search that found it still shown; a
/// person's mail becomes the search; a place, a Space or a command runs on the search there was.
fn act(ctx: Ctx, choice: Option<Choice>) {
    let Ctx {
        mut shell,
        mut pages,
        mut revision,
        mut side_hidden,
        spaces,
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
            shell.write().select(index);
            pages.set(1);
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
        // "New from template" lists the templates in the same panel rather than running anything.
        Choice::Pick(Pick::Action(CommandId::FromTemplate)) => {
            restore(shell, pages);
            if let Bar::Open(open) = &mut shell.write().bar {
                open.listing = BarListing::Templates(String::new());
                open.active = 0;
            }
        }
        Choice::Pick(Pick::Action(id)) => {
            restore(shell, pages);
            run_action(shell, pages, &mut revision, &mut side_hidden, spaces, id);
        }
    }
}

/// Run the command `id`. What it does is decided by the id alone; its words are not read.
fn run_action(
    mut shell: Signal<Shell>,
    mut pages: Signal<u32>,
    revision: &mut Signal<u64>,
    side_hidden: &mut Signal<bool>,
    spaces: Signal<crate::ui::space::Spaces>,
    id: CommandId,
) {
    match id {
        CommandId::Go(goes) => {
            let source = goes.source();
            let index = shell
                .read()
                .places
                .iter()
                .position(|one| one.source == source);
            if let Some(index) = index {
                shell.write().select(index);
                pages.set(1);
            }
            close(shell);
        }
        // Emptying goes to the bin first, so what is being emptied is what the window shows
        // behind the sheet that asks.
        CommandId::Empty(bin) => {
            let source = Source::Mail(place_filter(bin.role()));
            let index = shell
                .read()
                .places
                .iter()
                .position(|one| one.source == source);
            close(shell);
            if let Some(index) = index {
                shell.write().select(index);
                pages.set(1);
                let store = consume_context::<Arc<SqliteStore>>();
                super::destroy::ask_everything(&store, shell);
            }
        }
        // The entries that are a page of Settings open the window on that page.
        CommandId::SettingsAt(page) => {
            close(shell);
            super::settings_window::open_at(page);
        }
        CommandId::Compose => {
            let store = consume_context::<Arc<SqliteStore>>();
            if compose_new(&store, shell) {
                *revision += 1;
            }
            close(shell);
        }
        CommandId::Sync => {
            super::fetching::sync_now(&shell.read());
            close(shell);
        }
        CommandId::ForwardAttached => {
            close(shell);
            // The conversation open behind the menu, as Forward's `f` takes it. A refusal (no
            // body yet, or a message rebuilt from its parts) is said where every other outcome
            // of a command is.
            let Some(open) = shell.peek().open else {
                super::motion::tell(
                    "Open a conversation to forward it as an attachment.".to_owned(),
                    super::motion::Follow::Nothing,
                );
                return;
            };
            let store = consume_context::<Arc<SqliteStore>>();
            match start_composing(&store, open, Composes::ForwardAttached) {
                Ok(draft) => {
                    shell.write().compose(&draft);
                    *revision += 1;
                }
                Err(why) => super::motion::tell(
                    format!("Not forwarded as an attachment: {why}."),
                    super::motion::Follow::Nothing,
                ),
            }
        }
        CommandId::Print => {
            close(shell);
            let open = shell.read().open;
            if let Some(job) = super::print::job_for(open) {
                super::print::print(job);
            }
        }
        CommandId::Sidebar => {
            side_hidden.set(!side_hidden());
            close(shell);
        }
        CommandId::AddAccount => {
            close(shell);
            super::add_account::open();
        }
        CommandId::Import => {
            close(shell);
            super::files::open_import(shell);
        }
        CommandId::Export => {
            // Closed first: the sheet opens on the search the window is showing, not on what
            // was typed into this menu.
            close(shell);
            super::files::open_export(shell);
        }
        CommandId::NewView => {
            // Closed first, like Export: the view starts from the search the window shows.
            close(shell);
            let search = shell.peek().search.clone();
            super::views::open_new(shell, &search);
        }
        CommandId::Connections => {
            close(shell);
            super::doctor::open(shell);
        }
        CommandId::Settings => {
            close(shell);
            super::settings_window::open();
        }
        CommandId::Theme(look) => {
            let theme: ds::prelude::Theme = match look {
                Look::Light => ds::prelude::Theme::Light,
                Look::Dark => ds::prelude::Theme::Dark,
                Look::System => ds::prelude::Theme::System,
            };
            // The theme is the current Space's now, so this is a change to that Space. The
            // window's `Ds` root reads it from the Spaces; there is nothing to repaint by hand.
            let mut spaces = spaces;
            {
                let mut all = spaces.write();
                let current = all.current().id;
                all.edit(current, |space| space.look.theme = theme);
            }
            super::frame::keep(&spaces.read());
            close(shell);
        }
        // Listed in the panel itself, by `act`; run here it has nothing to do.
        CommandId::FromTemplate => close(shell),
    }
}

#[cfg(test)]
mod bar_tests;
#[cfg(test)]
pub(in crate::ui) mod pictures;
#[cfg(test)]
pub(in crate::ui) mod tests;
#[cfg(test)]
mod window_tests;
