//! The search bar's panel, as data: the rows it lists for what it lists, and the answer they
//! are read from, searched for off the thread that draws while the panel is up.

use super::super::debounce::{Settled, use_debounced};
use super::super::menu::{MenuItem, Right};
use super::items::{restate_sidebar, rows_of, search_now};
use super::sections::{Destination, TEMPLATES, places_for, sections};
use crate::ui::space::Spaces;
use crate::ui::today::Today;
use crate::ui::view::{BarListing, Shell, Source, place_key};
use chrono::Utc;
use dioxus::prelude::*;
use ds::prelude::*;
use mail_client::Client;
use mail_core::search::Results;
use std::collections::HashMap;

/// What the panel's rows are read from besides the answer.
pub(super) struct World<'a> {
    pub shell: &'a Shell,
    pub spaces: &'a Spaces,
    pub client: &'a Client,
    /// The window's keymap, for the hints beside the commands.
    pub keymap: &'a chordkit::Keymap,
    /// What was opened and put aside, for Recent while nothing is typed.
    pub today: &'a Today,
    /// How the person keeps the hours, for the times Recent's rows say.
    pub hours: mail_core::when::Clock,
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
        client,
        keymap,
        today,
        hours,
    } = world;
    match listing {
        BarListing::Templates(typed) => super::templates::rows(client, typed)
            .into_iter()
            .map(|row| MenuItem {
                group: Some(TEMPLATES.to_owned()),
                right: Right::None,
                ..row
            })
            .collect(),
        BarListing::Search => {
            let query = drawn.settled.text.as_str();
            let mut rows: Vec<MenuItem> = rows_of(&drawn.results, &drawn.names, query, keymap)
                .into_iter()
                .map(|row| restate_sidebar(row, sidebar, query))
                .collect();
            // Nothing typed: Recent leads, where the person was.
            if query.trim().is_empty() {
                let mut recent = super::recent::rows(
                    client,
                    today,
                    spaces.current().id,
                    chrono::Utc::now(),
                    *hours,
                );
                recent.append(&mut rows);
                rows = recent;
            }
            // Scheduled is a place only while a send waits, as the sidebar draws it.
            let waiting = query.trim().is_empty()
                && !crate::ui::compose::scheduled(client, &shell.scope).is_empty();
            let places: Vec<Destination> = shell
                .places
                .iter()
                .map(|place| Destination {
                    name: place.name.clone(),
                    icon: crate::ui::sidebar::place_icon(place),
                    // The VIPs, like the sidebar, only while someone is one.
                    at_rest: place_key(place) == format!("place:{}", place.name)
                        && (waiting || place.source != Source::Scheduled)
                        && (!crate::ui::view::is_vips(place) || !shell.vips.is_empty()),
                })
                .collect();
            let names: Vec<String> = spaces.list().iter().map(|one| one.name.clone()).collect();
            let current = spaces.index_of(spaces.current().id).unwrap_or(0);
            let found = places_for(query, &places, &names, current);
            sections(rows, found, query)
        }
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
    /// Search for `settled`. Blocking: the client is read.
    fn of(client: &Client, settled: Settled) -> Self {
        let (results, names) = search_now(client, &settled.text, Utc::now());
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
        let client = crate::ui::live::client();
        async move {
            if drawn.peek().settled == settled {
                return;
            }
            let done = tokio::task::spawn_blocking(move || Drawn::of(&client, settled)).await;
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
