//! mailo's window, drawn with quire, over a [`mail_client::Client`] and nothing below it.
//!
//! This crate names no `mail-core`, `mail-store` or `mail-runtime` (`docs/mail-client.md`): what
//! the window shows of the mail it reads from the client as values, and what it changes it hands
//! the client as commands, so no frame can wait on the store and no part of the window can reach
//! one. Its tests seed a store as fixtures (dev-dependencies) and hand the window a client over it.
//!
//! [`ui`] is the window. Beside it are the settings files it and the command line read
//! ([`settings`]), another program's ask to open a conversation ([`handoff`]) and the settings
//! files found damaged at launch ([`damage`]). The seams with the platform that draw nothing, and
//! what is said of a finished import, export or push, are `mail-os`'s.

pub mod damage;
pub mod desktop;
pub mod handoff;
pub mod settings;
#[cfg(feature = "testing")]
pub mod testing;
pub mod ui;

/// The desktop entry's name (`packaging/org.quire.Mail.desktop`), which notifications name too:
/// the window's app id (the Wayland `app_id` and X11 `WM_CLASS`, so the desktop matches the window
/// to its entry), and the app the desktop's intent router knows (`mail_app::intents::APP`), so the
/// scope a launcher starts the window in names it.
pub const APP_ID: &str = "org.quire.Mail";
