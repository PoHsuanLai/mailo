//! The decisions of a mail client that need no I/O. Pure: values in, values out.
//!
//! Its own crate because none of it touches a store, a socket, a clock or the environment, and
//! because a decision made here is made once for every program that asks: the window, the
//! command line and the agent. What the decisions are made over is `mail-domain`; what acts on
//! them is `mail-core` and the crates under it.
//!
//! A module that needs the time takes it as a value the caller hands in; none reads a clock, the
//! environment or the store itself.

pub mod follow_up;
pub mod mailbox_filter;
pub mod otp;
pub mod password;
pub mod picture;
pub mod query;
pub mod recent;
pub mod remedy;
pub mod saved_search;
pub mod scope;
pub mod snooze;
pub mod trust;
pub mod undo;
pub mod when;

/// The README's examples, compiled and run as doctests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
