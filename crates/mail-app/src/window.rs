//! Opening the window: what only the program makes for it, handed to `mail-ui`.
//!
//! The window reads mail through one client and names no store, so the program makes that client
//! here, over the store it opened, reaching servers as [`mail_edge`] says, and hands it over
//! with whose mail it is. A test's window is handed a client of its own and never comes here.

use mail_core::SqliteStore;
use std::sync::Arc;

/// Open the window over `store`, already wearing `look` and `spaces`, open where `start` says.
/// Returns when the window closes, or at once when it could not open.
pub fn open(
    store: Arc<SqliteStore>,
    look: mail_ui_kit::view::Appearance,
    spaces: mail_ui_kit::space::Spaces,
    dirs: Option<mail_ui_kit::appearance::WindowDirs>,
    start: mail_ui::ui::ask::Open,
) -> Result<(), mail_ui::ui::LaunchError> {
    // The desktop launcher's copy of the recent conversations and the address book, kept while
    // the window runs, on threads of its own (`intents::index`).
    #[cfg(not(any(target_os = "macos", windows)))]
    mail_intents::index::start(store.clone());
    let profile = mail_edge::profile().id().clone();
    // On macOS the app goes on with no window open, checking for mail, until it is quit, and is
    // the one that does whether a window is open or not (`crate::background`): its scheduler
    // starts here, before the window, so the window finds it.
    let stays = crate::background::stays_open(crate::background::Os::HERE);
    if stays {
        crate::background::start(store.clone(), profile.clone());
    }
    // This thread is the one the window draws on, and what it reads it reads before it can draw
    // again: the last reader is kept for it, so it never waits behind a sync pass's reads or a
    // worker's snapshot.
    SqliteStore::reserve_reader_for_this_thread();
    // The one client every window reads mail through, reaching the servers as the program does.
    let client = mail_client::Client::open(store.clone(), mail_client::ClientConfig::default());
    client.reach(mail_client::Reach::new(|store| {
        mail_edge::mail(&store).with_junk(mail_ui_model::settings::junk_policy())
    }));
    let resident = mail_ui::ui::Resident {
        stays,
        watching_here: crate::background::watching_here,
        // The Dock's count, kept by the app itself, whether a window is open or not and whoever
        // watches: no other process can badge the tile.
        keep_badge: stays.then(|| {
            std::sync::Arc::new(move |badge| {
                let kept = mail_cli::session::keep_the_badge_over(
                    store.clone(),
                    store.file().map(Into::into),
                    badge,
                );
                if let Err(e) = kept {
                    eprintln!("the Dock's unread count is off: {e}");
                }
            })
                as std::sync::Arc<
                    dyn Fn(std::sync::Arc<dyn mail_os::launcher::Badge>) + Send + Sync,
                >
        }),
    };
    let program = mail_ui::ui::Program {
        client,
        profile,
        resident,
    };
    mail_ui::ui::run(program, look, spaces, dirs, start)
}
