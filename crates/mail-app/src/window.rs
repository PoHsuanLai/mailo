//! Opening the window: what only the program makes for it, handed to `mail-ui`.
//!
//! The window reads mail through one client and names no store, so the program makes that client
//! here, over the store it opened, reaching servers as [`crate::edge`] says, and hands it over
//! with whose mail it is. A test's window is handed a client of its own and never comes here.

use mail_core::SqliteStore;
use std::sync::Arc;

/// Open the window over `store`, already wearing `look` and `spaces`, open where `start` says.
/// Returns when the window closes, or at once when it could not open.
pub fn open(
    store: Arc<SqliteStore>,
    look: mail_ui::ui::view::Appearance,
    spaces: mail_ui::ui::space::Spaces,
    dirs: Option<mail_ui::ui::appearance::WindowDirs>,
    start: mail_ui::ui::Start,
) -> Result<(), mail_ui::ui::LaunchError> {
    // The desktop launcher's copy of the recent conversations and the address book, kept while
    // the window runs, on threads of its own (`intents::index`).
    #[cfg(not(any(target_os = "macos", windows)))]
    crate::intents::index::start(store.clone());
    // The one client every window reads mail through, reaching the servers as the program does.
    let client = mail_client::Client::open(store, mail_client::ClientConfig::default());
    client.reach(mail_client::Reach::new(|store| crate::edge::mail(&store)));
    let program = mail_ui::ui::Program {
        client,
        profile: crate::edge::profile().id().clone(),
    };
    mail_ui::ui::run(program, look, spaces, dirs, start)
}
