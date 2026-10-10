//! The search bar's panel, as data: the rows it lists for what it lists, and the answer they
//! are read from, searched for off the thread that draws while the panel is up.

use super::super::debounce::{Settled, use_debounced};
use super::super::menu::{MenuItem, Right};
use super::items::{commands, restate_sidebar, rows_of, search_now};
use super::sections::{Destination, TEMPLATES, places_for, sections};
use crate::ui::space::Spaces;
use crate::ui::view::{BarListing, Shell, Source};
use chrono::Utc;
use dioxus::prelude::*;
use ds::prelude::*;
use mail_core::SqliteStore;
use mail_core::search::Results;
use std::collections::HashMap;
use std::sync::Arc;

/// What the panel's rows are read from besides the answer.
pub(super) struct World<'a> {
    pub shell: &'a Shell,
    pub spaces: &'a Spaces,
    pub store: &'a SqliteStore,
    /// The window's keymap, for the hints beside the commands.
    pub keymap: &'a chordkit::Keymap,
}

/// The panel's rows for what it lists.
pub(super) fn panel_rows(
    world: &World,
    drawn: &Drawn,
    listing: &BarListing,
    sidebar: Shown,
) -> Vec<MenuItem> {
    let World {
        shell,
        spaces,
        store,
        keymap,
    } = world;
    match listing {
        BarListing::Templates(typed) => super::templates::rows(store, typed)
            .into_iter()
            .map(|row| MenuItem {
                group: Some(TEMPLATES.to_owned()),
                right: Right::None,
                ..row
            })
            .collect(),
        BarListing::Search => {
            let query = drawn.settled.text.as_str();
            let rows: Vec<MenuItem> = rows_of(&drawn.results, &drawn.names, query, keymap)
                .into_iter()
                .map(|row| restate_sidebar(row, sidebar, query))
                .collect();
            let offered: Vec<String> = commands().into_iter().map(|one| one.label).collect();
            let places: Vec<Destination> = shell
                .places
                .iter()
                .map(|place| Destination {
                    name: place.name.clone(),
                    icon: place_icon(&place.source),
                })
                .collect();
            let names: Vec<String> = spaces.list().iter().map(|one| one.name.clone()).collect();
            let current = spaces.index_of(spaces.current().id).unwrap_or(0);
            let found = places_for(query, &places, &names, current, &offered);
            sections(rows, found, query)
        }
    }
}

/// A place's glyph in the panel.
fn place_icon(source: &Source) -> Icon {
    match source {
        Source::Mail(_) => Icon::Folder,
        Source::Drafts => Icon::FilePen,
        Source::Saved(_) => Icon::Search,
        Source::Waiting => Icon::Clock,
        Source::History => Icon::RotateLeft,
    }
}

/// One answer, and the settled text it answers.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Drawn {
    pub settled: Settled,
    pub results: Results,
    pub names: HashMap<String, String>,
}

impl Default for Drawn {
    /// No answer yet: a generation no debounce hands out, so the first settled text, even an
    /// empty one, is searched for.
    fn default() -> Self {
        Self {
            settled: Settled {
                generation: u64::MAX,
                text: String::new(),
            },
            results: Results::default(),
            names: HashMap::new(),
        }
    }
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
}

/// The search behind the panel, mounted while it is up: the field's text, once still, searched
/// off the thread that draws, and drawn only if no newer keystroke replaced it.
#[component]
pub(super) fn Fetch(shell: Signal<Shell>, drawn: Signal<Drawn>) -> Element {
    let debounced = use_debounced(shell, |shell| shell.search.clone());
    let _fetch = use_resource(move || {
        let settled = debounced.settled.read().clone();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            if drawn.peek().settled == settled {
                return;
            }
            let done = tokio::task::spawn_blocking(move || Drawn::of(&store, settled)).await;
            if let Ok(done) = done
                && debounced.is_latest(done.settled.generation)
            {
                let mut drawn = drawn;
                drawn.set(done);
            }
        }
    });
    rsx! {}
}
