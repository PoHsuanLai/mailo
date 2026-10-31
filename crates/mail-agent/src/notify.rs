//! Where a notification goes: the desktop's service, and the clicks on what it shows.
//!
//! `mail_core::notify` decides what is said and hands it to a [`Notifier`](mail_core::notify::Notifier);
//! what implements it for a desktop is here.

#[cfg(all(feature = "desktop", any(target_os = "macos", test)))]
pub mod banner;
#[cfg(all(unix, not(target_os = "macos"), feature = "desktop"))]
pub mod click;
#[cfg(feature = "desktop")]
pub mod desktop;
