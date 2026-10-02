//! Fetching mail, as state machines.
//!
//! Each thing that can be fetched, and each way that can go wrong, is a value here rather than a
//! flag in a Dioxus signal: [`Link`] for an account's connection, [`FolderFetch`] for a folder
//! opened on demand, [`Body`] for a message body, [`Download`] for an attachment. Each has a pure
//! `step` from a state and an event to the next state and what the caller must do about it.
//!
//! Nothing in this module reads the clock, opens a socket or touches the UI. Time arrives as an
//! argument, effects leave as values, and the window and the runtime do the rest. What the
//! screen says about a set of links is derived, not stored: that is the window's
//! (`mail_app::ui::fetching`: `list_face` and `status_line`).

mod body;
mod download;
mod folder;
mod link;

pub use body::{Body, BodyEffect, BodyEvent};
pub use download::{Download, DownloadEffect, DownloadEvent};
pub use folder::{AGAIN, FolderEffect, FolderEvent, FolderFetch};
pub use link::{
    BACKOFF_CEILING, Count, Effect, Event, First, Link, Live, Pause, RETRY_NOW, Step, Trigger,
    Trouble, backoff, step,
};
