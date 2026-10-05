//! Mailo as a provider of the desktop's intents: `org.quire.Mail` on the session bus, answering
//! `org.quire.IntentProvider1` for what `dist/intents/org.quire.Mail.toml` declares.
//!
//! The desktop's intent router (docket's `intentd`) is how a companion, the launcher and
//! `quire-do` ask an app to do something without drawing its window: it reads the manifest,
//! checks policy and consent, and calls `Perform` on the app's bus name, starting the app by
//! D-Bus activation when it is not running. `mailo intents` is what that starts: a process with
//! no window, over the same store and the same [`mail_core::act`] the window acts with, so what an
//! agent archives is archived the way a click archives it.
//!
//! No crate of the router is depended on: it is not public, and the wire is small. [`wire`]
//! writes the JSON forms out, [`Provider`] is the typed side and [`serve`] the bus side. A change
//! the window makes is not heard here, and one made here reaches the window through its look at
//! the store (`ui::fetching::external`).

mod provider;
pub mod wire;

#[cfg(not(any(target_os = "macos", windows)))]
mod serve;

pub use provider::{Opener, Provider};
#[cfg(not(any(target_os = "macos", windows)))]
pub use serve::{ServeError, serve, serve_on};

/// The bus name and the app name in the manifest.
pub const APP: &str = "org.quire.Mail";

/// Where every provider serves `org.quire.IntentProvider1`.
pub const PATH: &str = "/org/quire/IntentProvider1";

/// The router's bus name: the only caller a provider answers.
pub const ROUTER: &str = "org.quire.Intents1";

/// The manifest, as installed under `$XDG_DATA_DIRS/quire/intents/`.
pub const MANIFEST: &str = include_str!("../../../../dist/intents/org.quire.Mail.toml");

#[cfg(all(test, not(any(target_os = "macos", windows))))]
mod tests;
