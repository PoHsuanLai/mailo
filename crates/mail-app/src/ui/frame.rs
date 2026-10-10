//! What the window loads before the first picture: the Spaces, Today, and where they live.
//!
//! How a Space is painted is `paint.rs`.

use crate::ui::appearance::WindowDirs;
use crate::ui::space::{self, Spaces};
use crate::ui::today::{self, Today};
use dioxus::prelude::*;
use mail_core::SqliteStore;
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
/// Called from the component, where the contexts are. A missing file is a first run: one Space
/// per account, saved when there is a config directory to save it in (`space::boot`). A window
/// handed no directories takes the Spaces it was handed, if any (a test's), else a first run.
pub(super) fn load_boot() -> Boot {
    let dirs = try_consume_dirs();
    let store = dioxus::prelude::consume_context::<Arc<SqliteStore>>();
    let ids = super::data::accounts(&store);
    // What mailo wrote before quire, read only: the window-wide theme a Space without one of its
    // own starts from.
    let legacy = dirs
        .as_ref()
        .map(|dirs| crate::ui::appearance::legacy(&dirs.config))
        .unwrap_or_default();
    let spaces = match (&dirs, dioxus::prelude::try_consume_context::<Spaces>()) {
        (None, Some(handed)) => handed,
        _ => space::boot(dirs.as_ref(), &ids, &legacy),
    };
    let mut today = dirs
        .as_ref()
        .map(|dirs| today::load(&dirs.state))
        .unwrap_or_default();
    if today.prune(today::at(chrono::Utc::now())) > 0
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
        && space::save(Some(&dirs), spaces).is_ok()
    {
        // The other windows wear the Space too, and read the file again.
        crate::ui::revisions::told_configuration();
    }
}

/// Follow the configuration files the other windows write (`revisions::Configured`): when one
/// writes `settings.toml`, `keyboard.json` or `spaces.json`, read them again into the window's
/// settings, `shell` and `spaces`. The main window's Spaces go through `handle`, which leaves
/// them alone while a part of a Space's menu is open, whose close writes them; another window's
/// only wear them. A keymap that differs from the one the window's `keys` hold, whether this
/// window's Keyboard page changed it or another's did, is handed to `keys` at once, so a
/// rebinding or a reset applies without reopening anything.
pub(super) fn use_followed_configuration(
    mut shell: Signal<crate::ui::view::Shell>,
    mut spaces: Signal<Spaces>,
    handle: Option<crate::ui::space::Handle>,
    keys: ds::prelude::Keys,
) {
    let mut applied = use_hook(|| CopyValue::new(shell.peek().keymap.overrides()));
    use_effect(move || {
        let overrides = shell.read().keymap.overrides();
        if *applied.peek() != overrides {
            applied.set(overrides.clone());
            keys.set_overrides(overrides);
        }
    });
    let configured = use_signal(|| 0u64);
    crate::ui::revisions::use_shared_configuration(configured);
    use_effect(move || {
        let _ = configured();
        crate::ui::prefs::reread();
        let Some(dirs) = try_consume_dirs() else {
            return;
        };
        let keymap = crate::ui::keymap::load(&dirs.config);
        if shell.peek().keymap != keymap {
            shell.write().keymap = keymap;
        }
        let Some(stored) = space::storage(Some(&dirs)).load_spaces(|_| {}) else {
            return;
        };
        match handle {
            Some(handle) => handle.reload(stored),
            None if *spaces.peek() != stored => spaces.set(stored),
            None => {}
        }
    });
}
