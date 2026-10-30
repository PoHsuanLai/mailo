//! The engines that run an account: sync passes over IMAP and POP3, the outbox, and the
//! scheduler that decides when each of them runs.
//!
//! They call the protocol drivers of `mail-runtime` (`fetch`, `graph`, `jmap`, `sieve`, ...) and
//! are called by `mail-core`; no driver calls an engine, which is why this is a crate over
//! `mail-runtime` and not a module in it. What a driver and an engine both speak of (the
//! report of a pass, how a wait ends) lives in `mail_runtime::pass`.

pub mod engine;
pub mod schedule;

pub use engine::{AccountEngine, Notify, PartBudget};
