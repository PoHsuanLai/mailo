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
pub mod editor;
pub mod environment;
pub mod error;
pub mod export;
pub mod folder;
pub mod follow_up;
pub mod import;
pub mod invite;
pub mod ipc;
pub mod mail;
pub mod message;
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
pub mod remedy;
pub mod rules;
pub mod scope;
pub mod search;
pub mod server_search;
pub mod smime;
pub mod snooze;
pub mod sync;
pub mod template;
pub mod transfer;
pub mod trust;
pub mod undo;
pub mod unsubscribe;
pub mod view;
pub mod views;
pub mod when;

pub use environment::{Environment, Program};
pub use error::{CoreError, TimeError};
pub use mail::{
    AccountOps, Clock, ContactOps, CryptoOps, DiscoverOps, FixedClock, Mail, RuleOps, SyncOps,
    SystemClock,
};
/// What the window tells the body pass it is reading and fetching, so the pass does not repeat it.
pub use mail_runtime::wanted;
/// The runtime types mail-core's API hands out and takes: re-exported, so a front end goes
/// through mail-core alone.
pub use mail_runtime::{
    AccountSecrets, Accountd, Arrival, ClientRegistry, Destination, KeyringSigningStore, Link,
    MapSigningStore, Searched, ServerHits, SigningStore, Transport, absorb, absorb_into, assemble,
    fetch, link, off_runtime, remote_image, schedule, sieve::Pushed,
};
/// The store a front end opens and hands to mail-core.
pub use mail_store::{SqliteStore, Store, StoreError};
pub use remedy::{Remedy, SignInWith};
