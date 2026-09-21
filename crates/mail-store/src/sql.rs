//! Compiling a [`Filter`] into SQL.
//!
//! This module and [`mail_domain::Filter::fit`] are **two implementations of one semantics**.
//! They will diverge, silently, and the symptom is "search quietly missed a message". The
//! defence is `tests/parity.rs`, which asserts `fit(f, ctx) == (id in sql(f))` over generated
//! filters and corpora. If that test is ever skipped or weakened, the bug ships.

use chrono::{DateTime, Utc};
use mail_domain::Filter;

/// A bound parameter. Deliberately not `String`: binding is the only thing standing between a
/// subject line and SQL injection, so values never reach the query text.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Text(String),
    Int(i64),
}

/// A compiled filter: one boolean expression, plus its parameters in positional order.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlFilter {
    /// A SQL boolean expression referring to the alias `ts` (see [`compile`]). Always
    /// parenthesised, so it can be concatenated into a larger `WHERE` without precedence
    /// surprises.
    pub where_clause: String,
    pub params: Vec<SqlValue>,
}

/// Compile a filter to SQL.
///
/// **The alias contract**: the expression may reference exactly one table alias, `ts`, bound to
/// `thread_summary`. Anything needing per-message data — `Filter::Text`, `Filter::HasLabel` —
/// must emit a self-contained `EXISTS (...)` correlated on `ts.thread`, so the caller never has
/// to manage joins and two compiled filters always compose under `AND`/`OR`.
///
/// `now` resolves [`Filter::SnoozeDue`]; it is never read from the clock here.
pub fn compile(_filter: &Filter, _now: DateTime<Utc>) -> SqlFilter {
    todo!("wave 2")
}
