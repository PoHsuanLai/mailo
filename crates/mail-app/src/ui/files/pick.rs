//! The native file dialog, beside a typed path.
//!
//! `rfd` through the desktop portal: the same crate, version and feature `dioxus-desktop`
//! already links for its own file inputs, so nothing new is built. The dialog blocks until it
//! is answered, so it is opened on a blocking thread, from the click.

use std::path::PathBuf;

use dioxus::prelude::*;

/// What to ask the dialog for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ask {
    /// One file: an mbox or a `.eml`.
    File,
    /// A directory: a Maildir, or where an export goes.
    Folder,
}

/// Open the dialog in `start`, and hand what was chosen to `chosen`. Nothing, when it is
/// cancelled.
pub(super) fn choose(ask: Ask, start: PathBuf, mut chosen: impl FnMut(PathBuf) + 'static) {
    spawn(async move {
        let picked = tokio::task::spawn_blocking(move || {
            let dialog = rfd::FileDialog::new().set_directory(start);
            match ask {
                Ask::File => dialog.pick_file(),
                Ask::Folder => dialog.pick_folder(),
            }
        })
        .await;
        if let Ok(Some(path)) = picked {
            chosen(path);
        }
    });
}
