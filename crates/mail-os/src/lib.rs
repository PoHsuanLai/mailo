//! mailo's seams with the operating system, which draw nothing: the launcher's unread count
//! ([`launcher`]), the login item that checks mail while mailo is quit ([`login_item`]), the
//! system's mail-link handler ([`mail_handler`]), the desktop's notification service ([`desktop`]) and what an operation says of itself ([`said`]).
//!
//! This crate names no dioxus, no quire and no `mail-ui`: the window draws over these seams and
//! the command line prints through them, and neither names the other.

pub mod desktop;
pub mod launcher;
pub mod login_item;
pub mod mail_handler;
pub mod said;
