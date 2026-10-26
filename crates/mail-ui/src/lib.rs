//! mailo's window, drawn with quire, over a [`mail_client::Client`] and nothing below it.
//!
//! This crate names no `mail-core`, `mail-store` or `mail-runtime` (`docs/mail-client.md`): what
//! the window shows of the mail it reads from the client as values, and what it changes it hands
//! the client as commands, so no frame can wait on the store and no part of the window can reach
//! one. Its tests seed a store as fixtures (dev-dependencies) and hand the window a client over it.
//!
//! [`ui`] is the window. Beside it are the settings files it and the command line read
//! ([`settings`]), what is said of a finished import, export or push ([`said`]), and the window's
//! seams with the platform: the launcher's unread count ([`launcher`]), the system's mail-link
//! handler ([`mail_handler`]), another program's ask to open a conversation ([`handoff`]) and the
//! settings files found damaged at launch ([`damage`]).

pub mod damage;
pub mod handoff;
pub mod launcher;
pub mod mail_handler;
pub mod said;
pub mod settings;
pub mod ui;

/// The desktop entry's name (`packaging/org.quire.Mail.desktop`): the window's app id, and the
/// app the desktop's intent router knows (`ui::native::APP_ID`).
pub const APP_ID: &str = "org.quire.Mail";
