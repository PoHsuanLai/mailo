//! Downloads: every file the window saves, behind the Downloads button at the foot of the
//! sidebar, as a browser keeps the files it downloaded.
//!
//! Each place that writes a file (an attachment's Save or Download, the viewer's Save, an
//! invitation's `.ics`, a print saved as PDF, the vCard export) reports to the window's
//! [`Shelf`] through a [`Saving`]: taken in the handler that starts the work, ended when the
//! file is written. Saving shows as a row of its own while it runs, and the finished file joins
//! the stored [`log::Log`].
//!
//! The list is stored once for every window (`downloads.json` in the state directory), so a
//! file a message window saved is there too: the list is read again each time it opens.

mod log;
mod open;
mod panel;

pub(in crate::ui) use self::log::Origin;
pub(in crate::ui) use self::panel::DownloadsButton;

use self::log::{Entry, Log};
use crate::ui::appearance::WindowDirs;
use dioxus::prelude::*;
use mail_domain::MessageId;
use mail_store::{SqliteStore, Store as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the Downloads button and its list draw.
#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::ui) struct Shelf {
    /// The saved files, newest first.
    pub log: Log,
    /// Files being saved now, oldest first.
    pub going: Vec<Going>,
    /// Whether a file was saved since the list was last opened: the button's dot.
    pub unseen: bool,
    next: u64,
}

/// A file being saved: a download from the server can take a while.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Going {
    pub id: u64,
    pub name: String,
}

/// Provide the window's [`Shelf`], read from the state directory once.
pub(in crate::ui) fn use_provide_shelf(dirs: Option<&WindowDirs>) -> Signal<Shelf> {
    let state = dirs.map(|dirs| dirs.state.clone());
    use_context_provider(move || {
        Signal::new(Shelf {
            log: state.as_deref().map(log::load).unwrap_or_default(),
            ..Shelf::default()
        })
    })
}

/// One save's report to the Downloads list. Taken in the handler that starts the save, where
/// the window's context is in reach, and ended wherever the save finishes.
#[derive(Debug, Clone)]
pub(in crate::ui) struct Saving {
    shelf: Option<Signal<Shelf>>,
    state: Option<PathBuf>,
    id: Option<u64>,
}

impl Saving {
    /// A save that may take a while: `name` shows as being saved until [`Saving::end`].
    pub(in crate::ui) fn begin(name: impl Into<String>) -> Saving {
        let mut saving = Saving::quick();
        if let Some(mut shelf) = saving.shelf {
            let mut shelf = shelf.write();
            let id = shelf.next;
            shelf.next += 1;
            shelf.going.push(Going {
                id,
                name: name.into(),
            });
            saving.id = Some(id);
        }
        saving
    }

    /// A save that is over as soon as it starts: nothing shows until [`Saving::end`].
    pub(in crate::ui) fn quick() -> Saving {
        Saving {
            shelf: try_consume_context::<Signal<Shelf>>(),
            state: try_consume_context::<WindowDirs>().map(|dirs| dirs.state),
            id: None,
        }
    }

    /// The save is over: `saved` is the file written, if one was. Kept in the stored list and
    /// shown first in this window's.
    pub(in crate::ui) fn end(self, saved: Option<&Path>, origin: Option<Origin>) {
        if let (Some(mut shelf), Some(id)) = (self.shelf, self.id) {
            shelf.write().going.retain(|going| going.id != id);
        }
        let Some(path) = saved else {
            return;
        };
        let entry = Entry {
            path: path.to_owned(),
            at: chrono::Utc::now(),
            bytes: std::fs::metadata(path).map_or(0, |meta| meta.len()),
            origin,
        };
        // Read again before the write, so what another window saved meanwhile is kept.
        let mut stored = match &self.state {
            Some(state) => log::load(state),
            None => self
                .shelf
                .map(|shelf| shelf.peek().log.clone())
                .unwrap_or_default(),
        };
        stored.record(entry);
        if let Some(state) = &self.state {
            let _ = log::save(state, &stored);
        }
        if let Some(mut shelf) = self.shelf {
            let mut shelf = shelf.write();
            shelf.log = stored;
            shelf.unseen = true;
        }
    }
}

/// The message a saved file came out of, as the list names it; `None` when it is not stored.
pub(in crate::ui) fn origin(store: &SqliteStore, message: MessageId) -> Option<Origin> {
    store.message(message).ok().map(|m| Origin {
        thread: m.thread,
        subject: m.subject,
    })
}

/// The saved list as stored now, for a list being opened: another window may have added to it.
fn reread(mut shelf: Signal<Shelf>) {
    if let Some(dirs) = try_consume_context::<WindowDirs>() {
        let stored = log::load(&dirs.state);
        if stored != shelf.peek().log {
            shelf.write().log = stored;
        }
    }
}

/// Forget `path`, or every file when `None`; the files stay where they are.
fn forget(mut shelf: Signal<Shelf>, path: Option<&Path>) {
    let dirs = try_consume_context::<WindowDirs>();
    let mut stored = match &dirs {
        Some(dirs) => log::load(&dirs.state),
        None => shelf.peek().log.clone(),
    };
    match path {
        Some(path) => stored.remove(path),
        None => stored = Log::default(),
    }
    if let Some(dirs) = &dirs {
        let _ = log::save(&dirs.state, &stored);
    }
    shelf.write().log = stored;
}

/// What opens a saved file and shows one in its folder. The window uses the system's; a test
/// provides its own as a root context and nothing is launched.
#[derive(Clone)]
pub struct Opener {
    open: Arc<FileAct>,
    reveal: Arc<FileAct>,
}

type FileAct = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

impl Opener {
    /// `open` opens a file in its app; `reveal` shows it in its folder.
    pub fn new(
        open: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static,
        reveal: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static,
    ) -> Opener {
        Opener {
            open: Arc::new(open),
            reveal: Arc::new(reveal),
        }
    }
}

impl Default for Opener {
    fn default() -> Self {
        Opener {
            open: Arc::new(open::open),
            reveal: Arc::new(open::reveal),
        }
    }
}

impl std::fmt::Debug for Opener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Opener")
    }
}

fn opener() -> Opener {
    try_consume_context::<Opener>().unwrap_or_default()
}

#[cfg(test)]
mod tests;
