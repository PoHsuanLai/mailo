//! The search bar in the list's toolbar: one field that searches the list, and a panel under
//! it of the mail, commands, places and people the text names. What the ⌘K menu offered, it
//! offers, matched and ranked as the menu did.
//!
//! The rows are built in [`items`] and regrouped into the panel's sections in [`sections`]; the
//! field and its panel are [`bar`], its keys [`keys`]; this file is what a pick does.

mod bar;
mod items;
mod keys;
mod panel;
pub(in crate::ui) mod people;
mod sections;
mod templates;

#[cfg(test)]
pub(in crate::ui) use bar::LABEL;
pub(in crate::ui) use bar::{SearchBar, press_elsewhere, summon};
pub(in crate::ui) use items::avatar_color;

use super::ops::{Composes, start_composing, start_new};
use crate::ui::view::{Bar, BarListing, PageMenu, SettingsPage, Shell};
use bar::Ctx;
use dioxus::prelude::*;
use items::Pick;
use mail_store::SqliteStore;
use sections::Choice;
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
        Choice::Space(index) => {
            restore(shell, pages);
            close(shell);
            super::switch::go(spaces, shell, pages, index);
        }
        // "New from template" lists the templates in the same panel rather than running anything.
        Choice::Pick(Pick::Action(label)) if label == templates::ACTION => {
            restore(shell, pages);
            if let Bar::Open(open) = &mut shell.write().bar {
                open.listing = BarListing::Templates(String::new());
                open.active = 0;
            }
        }
        Choice::Pick(Pick::Action(label)) => {
            restore(shell, pages);
            run_action(
                shell,
                pages,
                &mut revision,
                &mut side_hidden,
                spaces,
                &label,
            );
        }
    }
}

/// The page of Settings a command's entry opens, when it is one.
fn settings_page_of(label: &str) -> Option<SettingsPage> {
    match label {
        "General Settings" => Some(SettingsPage::General),
        "Accounts Settings" => Some(SettingsPage::Accounts),
        "Contacts Settings" => Some(SettingsPage::Contacts),
        "Rules Settings" => Some(SettingsPage::Rules),
        "Keys and Certificates Settings" => Some(SettingsPage::Keys),
        "Keyboard Shortcuts Settings" => Some(SettingsPage::Keyboard),
        _ => None,
    }
}

fn run_action(
    mut shell: Signal<Shell>,
    mut pages: Signal<u32>,
    revision: &mut Signal<u64>,
    side_hidden: &mut Signal<bool>,
    spaces: Signal<crate::ui::space::Spaces>,
    label: &str,
) {
    if let Some(place) = label.strip_prefix("Go to ") {
        let index = shell.read().places.iter().position(|one| one.name == place);
        if let Some(index) = index {
            shell.write().select(index);
            pages.set(1);
        }
        close(shell);
        return;
    }
    // Emptying goes to the bin first, so what is being emptied is what the window shows behind
    // the sheet that asks.
    if let Some(bin) = label
        .strip_prefix("Empty ")
        .and_then(|rest| rest.strip_suffix('…'))
    {
        let index = shell.read().places.iter().position(|one| one.name == bin);
        close(shell);
        if let Some(index) = index {
            shell.write().select(index);
            pages.set(1);
            let store = consume_context::<Arc<SqliteStore>>();
            super::destroy::ask_everything(&store, shell);
        }
        return;
    }
    // The entries that are a page of Settings open the window on that page.
    if let Some(page) = settings_page_of(label) {
        close(shell);
        super::settings_window::open_at(page);
        return;
    }
    match label {
        "Compose" => {
            let store = consume_context::<Arc<SqliteStore>>();
            let known = shell.peek().accounts.clone();
            match start_new(&store, &known) {
                Ok(draft) => {
                    shell.write().compose(&draft);
                    *revision += 1;
                }
                Err(why) => eprintln!("compose: {why}"),
            }
            close(shell);
        }
        "Sync now" => {
            super::fetching::sync_now(&shell.read());
            close(shell);
        }
        "Forward as attachment" => {
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
        "Print conversation" => {
            close(shell);
            let open = shell.read().open;
            if let Some(job) = super::print::job_for(open) {
                super::print::print(job);
            }
        }
        "Hide sidebar" => {
            side_hidden.set(!side_hidden());
            close(shell);
        }
        "Add account…" => {
            close(shell);
            super::add_account::open();
        }
        "Import mail…" => {
            close(shell);
            super::files::open_import(shell);
        }
        "Export mail…" => {
            // Closed first: the sheet opens on the search the window is showing, not on what
            // was typed into this menu.
            close(shell);
            super::files::open_export(shell);
        }
        "New view…" => {
            // Closed first, like Export: the view starts from the search the window shows.
            close(shell);
            let search = shell.peek().search.clone();
            super::views::open_new(shell, &search);
        }
        "Connection Doctor" => {
            close(shell);
            super::doctor::open(shell);
        }
        "Settings…" => {
            close(shell);
            super::settings_window::open();
        }
        "Theme light" | "Theme dark" | "Theme system" => {
            let theme: ds::prelude::Theme = match label {
                "Theme light" => ds::prelude::Theme::Light,
                "Theme dark" => ds::prelude::Theme::Dark,
                _ => ds::prelude::Theme::System,
            };
            // The theme is the current Space's now, so this is a change to that Space. The
            // window's `Ds` root reads it from the Spaces; there is nothing to repaint by hand.
            let mut spaces = spaces;
            {
                let mut all = spaces.write();
                let current = all.current;
                match all.spaces.get_mut(current) {
                    Some(space) => space.look.theme = theme,
                    None => return close(shell),
                }
            }
            super::frame::keep(&spaces.read());
            close(shell);
        }
        _ => close(shell),
    }
}

#[cfg(test)]
mod bar_tests;
#[cfg(test)]
pub(in crate::ui) mod pictures;
#[cfg(test)]
pub(in crate::ui) mod tests;
