//! The logic of mailo: everything that is neither the window nor the terminal.
//!
//! Sync and the fetch state machines, accounts and their discovery, composing, rules, contacts,
//! search, the switches kept in the config directory, and the rest, over the pure crates
//! (`mail-domain`, `mail-mime`, `mail-proto`, `mail-pim`) and the I/O ones (`mail-store`,
//! `mail-runtime`). `mail-app` is its only user, and draws two front-ends over it: the window
//! (`mail_ui::ui`) and the command line (`mail_app::cli`).
//!
//! The way in is [`Mail`]: one handle over the store, the secrets, the environment and the clock,
//! with the operations that wait on a network as `async` methods grouped by what they are for.
//! This crate starts no runtime and reads no environment variable; the front-end owns both and
//! hands them in.
//!
//! This crate never depends on a toolkit that draws (`dioxus`, `ds`, `ds-settings`,
//! `ds-blitz`): `scripts/check-boundary.sh` fails the build if it does. A function here returns
//! a value for a front-end to word, not the words of one; the modules that still return
//! terminal prose are named in `scripts/core-prose-allowlist.txt`, to be converted.

pub mod account;
pub mod act;
pub mod attach;
pub mod attachment_image;
pub mod auth;
pub mod bimi;
pub mod compose;
pub mod config;
pub mod contacts;
pub mod conversation;
pub mod discover;
pub mod environment;
pub mod erase;
pub mod error;
pub mod export;
pub mod findable;
pub mod folder;
pub mod follow_up;
pub mod history;
pub mod html_document;
pub mod identity;
pub mod import;
pub mod invite;
pub mod ipc;
pub mod junk;
pub mod mail;
pub mod mailbox_filter;
pub mod message;
pub mod notify;
pub mod offline;
pub mod otp;
pub mod password;
pub mod pgp;
pub mod profile;
pub mod protect;
pub mod provider;
pub mod query;
pub mod receipt;
pub mod recent;
pub mod remedy;
pub mod rich_text;
pub mod rules;
pub mod saved_search;
pub mod scope;
pub mod search;
pub mod server_search;
pub mod signature;
pub mod single;
pub mod smime;
pub mod snooze;
pub mod sync;
pub mod template;
pub mod transfer;
pub mod trust;
pub mod undo;
pub mod unsubscribe;
pub mod vip;
pub mod when;

pub use environment::{Environment, Program};
pub use error::{CoreError, TimeError};
pub use mail::{
    AccountOps, Clock, ContactOps, CryptoOps, DiscoverOps, FixedClock, Mail, OAuthClients, RuleOps,
    SyncOps, SystemClock,
};
/// An account's storage as its server counts it ([`SyncOps::quota`]).
pub use mail_proto::Storage;
/// The doubles at the runtime's seams, for a front end's tests: an in-memory transport, secrets,
/// signing store and OAuth clients that sign in nowhere.
#[cfg(feature = "testing")]
pub use mail_runtime::testing;
/// What the window tells the body pass it is reading and fetching, so the pass does not repeat it.
pub use mail_runtime::wanted;
/// The runtime types mail-core's API hands out and takes: re-exported, so a front end goes
/// through mail-core alone.
pub use mail_runtime::{
    AccountSecrets, Arrival, ClientRegistry, Destination, KeyringSigningStore, Logged, NoKeyring,
    Searched, ServerHits, SigningStore, Transport, absorb, absorb_into, assemble, fetch,
    graph::push::RelayRefused, off_runtime, remote_image, schedule, sieve::Pushed,
};
/// The store a front end opens and hands to mail-core.
pub use mail_store::{AccountPrefs, SqliteStore, Store, StoreError, Syncing};
pub use remedy::{Remedy, SignInWith};

#[cfg(test)]
mod support_tests;
