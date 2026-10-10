//! The logic of mailo: everything that is neither the window nor the terminal.
//!
//! Sync and the fetch state machines, accounts and their discovery, composing, rules, contacts,
//! search, the switches kept in the config directory, and the rest, over the pure crates
//! (`mail-domain`, `mail-mime`, `mail-proto`, `mail-pim`) and the I/O ones (`mail-store`,
//! `mail-runtime`). `mail-app` is its only user, and draws two front-ends over it: the window
//! (`mail_app::ui`) and the command line (`mail_app::cli`).
//!
//! The way in is [`Mail`]: one handle over the store, the link to accountd, the environment and
//! the clock, with the operations that wait on a network as `async` methods grouped by what they
//! are for. This crate starts no runtime and reads no environment variable; the front-end owns
//! both and hands them in.
//!
//! This crate never depends on a toolkit that draws (`dioxus`, `ds`, `ds-settings`,
//! `ds-blitz`): `scripts/check-boundary.sh` fails the build if it does. A function here returns
//! a value for a front-end to word, not the words of one; the modules that still return
//! terminal prose are named in `scripts/core-prose-allowlist.txt`, to be converted.

pub mod account;
pub mod act;
pub mod attach;
pub mod auth;
pub mod bimi;
pub mod compose;
pub mod config;
pub mod contacts;
pub mod discover;
pub mod environment;
pub mod error;
pub mod export;
pub mod fetch;
pub mod folder;
pub mod follow_up;
pub mod import;
pub mod invite;
pub mod ipc;
pub mod mail;
pub mod notify;
pub mod offline;
pub mod password;
pub mod pgp;
pub mod place;
pub mod preview;
pub mod print;
pub mod provider;
pub mod query;
pub mod receipt;
pub mod rules;
pub mod search;
pub mod server_search;
pub mod smime;
pub mod snooze;
pub mod sync;
pub mod template;
pub mod trust;
pub mod undo;
pub mod unsubscribe;
pub mod when;

pub use environment::Environment;
pub use error::{CoreError, TimeError, UsageError};
pub use mail::{
    AccountOps, Clock, ContactOps, CryptoOps, DiscoverOps, FixedClock, Mail, RuleOps, SyncOps,
    SystemClock,
};
