//! The per-user watch's lock and door, and the desktop notifications that announce new mail.
//!
//! [`ipc`] is the conversation the window and the command line have with the one `mailo watch`
//! (and the daemon) of a user: its lock, its door, the requests and answers. [`notify`] is where
//! a notification goes once `mail_core::notify` has decided to raise it: the desktop's service
//! on Linux and Windows, and on macOS the banners the app posts, handed in through
//! `notify::banner::Posts`.
//!
//! The decisions stay in `mail-core` (`mail_core::notify`: which mail deserves a word, how many
//! words a burst gets, the [`Notifier`](mail_core::notify::Notifier) seam the watch announces
//! through), so this crate depends on it and it never names this one.

pub mod ipc;
pub mod notify;
