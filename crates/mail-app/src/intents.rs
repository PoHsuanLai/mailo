//! `mailo intents`: what the router starts by D-Bus activation, with no window and the same store.
//!
//! The provider and the bus are `mail-intents`'; the command is `mail-cli`'s word, which names
//! neither crate's other side, so the process is where the two meet.

use mail_core::SqliteStore;
use std::sync::Arc;

/// Answer the router over `store` until the bus lets go. A second one finds the name taken and ends
/// quietly (exit 0), so a start that races another is not a failure.
pub fn serve(store: Arc<SqliteStore>) -> Result<(), String> {
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        use mail_intents::{Opener, Provider, ServeError};
        let provider = Provider::new(store, mail_edge::signing(), Opener::window())
            .with_hours(mail_edge::environment().hours);
        match mail_intents::serve(provider) {
            Ok(()) | Err(ServeError::Taken) => Ok(()),
            Err(why) => Err(why.to_string()),
        }
    }
    #[cfg(any(target_os = "macos", windows))]
    {
        let _ = store;
        Err("mailo intents answers on the Linux session bus, which this system has not".to_owned())
    }
}
