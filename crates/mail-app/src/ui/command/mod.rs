//! The Ctrl T menu: a field, the operator chips, and the grouped results.
//!
//! The rows are built in [`items`]; this file is the overlay and what a pick does. The search
//! runs off the thread that draws, once the field has been still for [`super::debounce::QUIET`],
//! and an answer to text a newer keystroke has replaced is dropped. What is drawn is always one
//! answer to one query, so Enter picks from the rows on screen, not from a query run afresh.

mod items;
pub(in crate::ui) mod people;
mod templates;

pub(in crate::ui) use items::avatar_color;

use super::debounce::{Settled, use_debounced};
use super::menu::quire_groups;
use super::ops::{Composes, start_composing, start_new};
use crate::search::Results;
use crate::view::{PageMenu, Shell, Theme};
use chrono::Utc;
use dioxus::prelude::*;
use items::{Pick, interpret, rows_of, search_now, tokens};
use mail_store::SqliteStore;
use std::collections::HashMap;
use std::sync::Arc;

/// The centred overlay: quire's `CommandPalette`, rising opaque. Open while `shell.command` is
/// `Some`. It floats over the window on quire's palette layer, so the keys typed into it never
/// reach the window's shortcuts.
#[component]
pub(super) fn CommandMenu(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    revision: Signal<u64>,
    side_hidden: Signal<bool>,
    sync_state: Signal<crate::view::SyncState>,
    spaces: Signal<crate::space::Spaces>,
) -> Element {
    let query = shell.read().command.clone().unwrap_or_default();
    let debounced = use_debounced(shell, |shell| shell.command.clone().unwrap_or_default());
    let mut drawn = use_signal(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        Drawn::of(&store, debounced.settled.peek().clone())
    });
    let _fetch = use_resource(move || {
        let settled = debounced.settled.read().clone();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            // The first frame already drew this generation, on the thread that draws.
            if drawn.peek().settled == settled {
                return;
            }
            let done = tokio::task::spawn_blocking(move || Drawn::of(&store, settled)).await;
            if let Ok(done) = done
                && debounced.is_latest(done.settled.generation)
            {
                drawn.set(done);
            }
        }
    });
    let shown = drawn.read();
    let items = rows_of(&shown.results, &shown.names, &shown.settled.text);
    drop(shown);
    let chips = tokens(&query);
    let placeholder = "Search mail, people, actions · try from:dana or has:attachment".to_owned();
    // "New from template" lists the templates in this same overlay rather than running anything.
    let mut listing = use_signal(|| Listing::Search);
    let mut choose = move |pick: Option<Pick>| match pick {
        Some(Pick::Action(label)) if label == templates::ACTION => {
            listing.set(Listing::Templates);
            shell.write().command = Some(String::new());
        }
        pick => act(
            shell,
            pages,
            revision,
            side_hidden,
            sync_state,
            spaces,
            pick,
        ),
    };
    if listing() == Listing::Templates {
        return rsx! { templates::TemplateMenu { shell, revision } };
    }
    let groups = quire_groups(&items, ds::AvatarSize::Size34, None);
    rsx! {
        ds::CommandPalette::<String> {
            label: "Search and commands".to_owned(),
            placeholder,
            query,
            tokens: chips,
            groups,
            empty: "Nothing matches.".to_owned(),
            entrance: ds::PaletteEntrance::Opaque,
            oninput: move |value| {
                shell.write().command = Some(value);
            },
            onpick: move |key: String| {
                let pick = drawn.read().pick(&key);
                choose(pick);
            },
            onclose: move |()| close(shell),
            onkey: move |event: KeyboardEvent| toggle_key(&event, shell),
        }
    }
}

/// Ctrl T in the palette's field closes it, as Ctrl T in the window opens it. The field holds
/// the keyboard while the palette is up, so the window's own shortcut never hears it.
fn toggle_key(event: &KeyboardEvent, shell: Signal<Shell>) {
    let key = event.key().to_string();
    if event.modifiers().ctrl() && (key == "t" || key == "T") {
        event.prevent_default();
        close(shell);
    }
}

/// What the overlay is listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    /// Mail, people and actions for the query.
    Search,
    /// Templates, after "New from template".
    Templates,
}

/// One answer, and the settled text it answers.
#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    settled: Settled,
    results: Results,
    names: HashMap<String, String>,
}

impl Drawn {
    /// Search for `settled`. Blocking: the store is read.
    fn of(store: &SqliteStore, settled: Settled) -> Self {
        let (results, names) = search_now(store, &settled.text, Utc::now());
        Self {
            settled,
            results,
            names,
        }
    }

    /// What the row keyed `key` does, read back against this answer.
    fn pick(&self, key: &str) -> Option<Pick> {
        interpret(&self.results, key)
    }
}

fn close(mut shell: Signal<Shell>) {
    shell.write().command = None;
    shell.write().page_menu = PageMenu::Closed;
    crate::ui::host::Host::focus_app();
}

fn act(
    mut shell: Signal<Shell>,
    mut pages: Signal<u32>,
    mut revision: Signal<u64>,
    mut side_hidden: Signal<bool>,
    mut sync_state: Signal<crate::view::SyncState>,
    spaces: Signal<crate::space::Spaces>,
    pick: Option<Pick>,
) {
    let Some(pick) = pick else {
        return;
    };
    match pick {
        Pick::Open(id) => {
            shell.write().open(id);
            close(shell);
        }
        Pick::From(email) => {
            shell.write().search = format!("from:{email}");
            pages.set(1);
            close(shell);
        }
        Pick::Action(label) => run_action(
            shell,
            pages,
            &mut revision,
            &mut side_hidden,
            &mut sync_state,
            spaces,
            &label,
        ),
    }
}

fn run_action(
    mut shell: Signal<Shell>,
    mut pages: Signal<u32>,
    revision: &mut Signal<u64>,
    side_hidden: &mut Signal<bool>,
    sync_state: &mut Signal<crate::view::SyncState>,
    spaces: Signal<crate::space::Spaces>,
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
            if sync_state.read().may_start() {
                sync_state.set(crate::view::SyncState::Running);
                let store = consume_context::<Arc<SqliteStore>>();
                let mut sync_state = *sync_state;
                let mut revision = *revision;
                spawn(async move {
                    let done = tokio::task::spawn_blocking(move || {
                        crate::sync::run(store, chrono::Utc::now())
                    })
                    .await;
                    sync_state.set(match done {
                        Ok(result) => crate::view::synced(result.map(|ran| ran.text)),
                        Err(e) => crate::view::synced(Err(format!("the sync pass stopped: {e}"))),
                    });
                    revision += 1;
                });
            }
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
        "Contacts" => {
            close(shell);
            super::contacts::open(shell);
        }
        "Add account…" => {
            close(shell);
            super::add_account::open(shell);
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
        "Rules…" => {
            close(shell);
            super::rules::open(shell);
        }
        "Keys and certificates…" => {
            close(shell);
            super::pgp::keys::open(shell);
        }
        "Theme light" | "Theme dark" | "Theme system" => {
            let theme = match label {
                "Theme light" => Theme::Light,
                "Theme dark" => Theme::Dark,
                _ => Theme::System,
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
pub(in crate::ui) mod tests;
