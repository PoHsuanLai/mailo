//! What AppKit tells a running mailo that winit does not pass on: a URL the system opens with it
//! (`mailto:`, `mailo:thread/…`) and a click on its Dock icon while it runs. And the one other
//! thing mailo asks of the system that only an unsafe call reaches: its login item ([`login`]),
//! and the banners it posts to the notification centre and the one delegate that hears their
//! clicks ([`banner`]).
//!
//! winit never registers an application delegate (its AppKit docs say so and show this very
//! pattern), so mailo may. Writing one takes `unsafe` (`objc2::define_class!`), which the rest of
//! the workspace forbids, hence this crate: a callback in, a safe function out. Empty off macOS.
//!
//! Two functions because they have different deadlines. [`hear_urls`] must run before the event
//! loop does, or the URL a cold launch carries is gone before anyone listens; [`hear_reopen`]
//! needs `NSApplication`, which winit makes only when its event loop is created.

#![cfg(target_os = "macos")]

pub mod banner;
mod delegate;
pub mod login;
mod urls;

pub use delegate::hear_reopen;
pub use urls::hear_urls;
