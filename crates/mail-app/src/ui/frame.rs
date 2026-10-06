//! What the window loads before the first picture: the Spaces, Today, and where they live.
//!
//! How a Space is painted is `paint.rs`.

use crate::ui::appearance::WindowDirs;
use crate::ui::space::{self, Spaces};
use crate::ui::today::{self, Today};
use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;

/// What the first render needs, and nothing it has to ask the disk for again.
#[derive(Clone)]
pub(super) struct Boot {
    pub spaces: Spaces,
    pub today: Today,
    pub dirs: Option<WindowDirs>,
}

/// Load Spaces and Today.
///
/// Called from the component, where the contexts are. A missing file is a first run:
/// one Space per account, saved when there is a config directory to save it in.
pub(super) fn load_boot() -> Boot {
    let dirs = try_consume_dirs();
    let store = dioxus::prelude::consume_context::<Arc<SqliteStore>>();
    let mut spaces = if let Some(dirs) = &dirs {
        space::load(&dirs.config)
    } else {
        dioxus::prelude::try_consume_context::<Spaces>().unwrap_or_default()
    };
    let ids = super::data::accounts(&store);
    if spaces.spaces.is_empty() {
        spaces = space::first_run(&ids);
        // What mailo wrote before quire, read only: the window-wide theme and motion a first
        // run's Spaces start from.
        let look = dirs
            .as_ref()
            .map(|dirs| crate::ui::appearance::legacy(&dirs.config))
            .unwrap_or_default();
        space::inherit(&mut spaces, &look);
        if let Some(dirs) = &dirs {
            let _ = space::save(&dirs.config, &spaces);
        }
    }
    let filled = spaces
        .spaces
        .get_mut(spaces.current)
        .is_some_and(|space| space::ensure_colors(space, &ids));
    if filled && let Some(dirs) = &dirs {
        let _ = space::save(&dirs.config, &spaces);
    }
    let mut today = dirs
        .as_ref()
        .map(|dirs| today::load(&dirs.state))
        .unwrap_or_default();
    let before = today.entries.len();
    today.prune(chrono::Utc::now());
    if today.entries.len() != before
        && let Some(dirs) = &dirs
    {
        let _ = today::save(&dirs.state, &today);
    }
    Boot {
        spaces,
        today,
        dirs,
    }
}

fn try_consume_dirs() -> Option<WindowDirs> {
    dioxus::prelude::try_consume_context::<WindowDirs>()
}

/// Write `spaces` to `spaces.json` when the window has a config directory.
///
/// Atomic, through `space::save`. A window launched with nowhere to write keeps its Spaces
/// for the session, and a file that cannot be written changes nothing on screen.
pub(super) fn keep(spaces: &Spaces) {
    if let Some(dirs) = try_consume_dirs()
        && space::save(&dirs.config, spaces).is_ok()
    {
        // The other windows wear the Space too, and read the file again.
        crate::ui::revisions::told_configuration();
    }
}

/// Follow the configuration files the other windows write (`revisions::Configured`): when one
/// writes `keyboard.json` or `spaces.json`, read them again into `shell` and `spaces`. The
/// Spaces are left alone while `editing` holds a draft of one, which closing keeps.
pub(super) fn use_followed_configuration(
    mut shell: Signal<crate::ui::view::Shell>,
    mut spaces: Signal<Spaces>,
    editing: Option<Signal<Option<crate::ui::space::edit::Draft>>>,
) {
    let configured = use_signal(|| 0u64);
    crate::ui::revisions::use_shared_configuration(configured);
    use_effect(move || {
        let _ = configured();
        let Some(dirs) = try_consume_dirs() else {
            return;
        };
        let keymap = crate::ui::keymap::load(&dirs.config);
        if shell.peek().keymap != keymap {
            shell.write().keymap = keymap;
        }
        if editing.is_some_and(|draft| draft.peek().is_some()) {
            return;
        }
        let stored = space::load(&dirs.config);
        if !stored.spaces.is_empty() && *spaces.peek() != stored {
            spaces.set(stored);
        }
    });
}
