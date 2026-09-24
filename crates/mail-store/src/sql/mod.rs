//! Compiling a [`Filter`] into SQL.
//!
//! This module and [`mail_domain::Filter::fit`] are **two implementations of one semantics**.
//! They will diverge, silently, and the symptom is "search quietly missed a message". The
//! defence is `tests/parity.rs`, which asserts `fit(f, ctx) == (id in sql(f))` over generated
//! filters and corpora. If that test is ever skipped or weakened, the bug ships.

use chrono::{DateTime, SecondsFormat, Utc};
use mail_domain::{DateRange, Filter, MailboxRole, TextMatch};
use serde::Serialize;

mod fts;
pub(crate) use fts::{MATCH_PREDICATE, match_needles, message_index};

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
/// `thread_summary`. Anything needing per-message data — [`Filter::Text`] — must emit a
/// self-contained `EXISTS (...)` correlated on `ts.thread`, so the caller never has to manage
/// joins and two compiled filters always compose under `AND`/`OR`. [`Filter::HasLabel`] reads
/// `ts.labels`, which is already the union over the thread.
///
/// `now` resolves [`Filter::SnoozeDue`]; it is never read from the clock here.
pub fn compile(filter: &Filter, now: DateTime<Utc>) -> SqlFilter {
    let mut params = Vec::new();
    let where_clause = predicate(filter, now, &mut params);
    SqlFilter {
        where_clause,
        params,
    }
}

fn predicate(filter: &Filter, now: DateTime<Utc>, params: &mut Vec<SqlValue>) -> String {
    match filter {
        Filter::All => "(1=1)".to_owned(),
        Filter::Nothing => "(1=0)".to_owned(),
        Filter::And(children) => combine(children, "AND", "(1=1)", now, params),
        Filter::Or(children) => combine(children, "OR", "(1=0)", now, params),
        Filter::Not(inner) => {
            let child = predicate(inner, now, params);
            format!("(NOT {child})")
        }
        Filter::Account(id) => {
            params.push(SqlValue::Text(id.to_string()));
            "(ts.account = ?)".to_owned()
        }
        Filter::InMailbox(role) => {
            params.push(SqlValue::Text(role_name(*role).to_owned()));
            "(EXISTS (SELECT 1 FROM json_each(ts.mailboxes) WHERE value = ?))".to_owned()
        }
        Filter::Read(state) => {
            params.push(SqlValue::Text(stored_json(state)));
            "(ts.read = ?)".to_owned()
        }
        Filter::Starred(star) => {
            params.push(SqlValue::Text(stored_json(star)));
            "(ts.star = ?)".to_owned()
        }
        Filter::HasLabel(id) => {
            params.push(SqlValue::Text(id.to_string()));
            "(EXISTS (SELECT 1 FROM json_each(ts.labels) WHERE value = ?))".to_owned()
        }
        // Uncorrelated, for the reason `fts::predicate` gives: the subquery names no outer row,
        // so SQLite runs it once — a search of `remote_map_identity` on its leading
        // `(account, mailbox)` columns, then each message by primary key — and tests each
        // thread against the result, rather than probing `remote_map` once per thread.
        Filter::InFolder(mailbox) => {
            params.push(SqlValue::Text(mailbox.account.to_string()));
            params.push(SqlValue::Text(mailbox.path.clone()));
            format!("(ts.thread IN ({IN_FOLDER}))")
        }
        Filter::From(m) => like_either("ts.from_email", "ts.from_name", m, params),
        Filter::To(m) => {
            let pattern = like_pattern(m);
            params.push(SqlValue::Text(pattern.clone()));
            params.push(SqlValue::Text(pattern));
            format!(
                "(EXISTS (SELECT 1 FROM json_each(ts.recipients) WHERE \
                 json_extract(value, '$.email') {LIKE} OR json_extract(value, '$.name') {LIKE}))"
            )
        }
        Filter::Subject(m) => {
            params.push(SqlValue::Text(like_pattern(m)));
            format!("(ts.subject {LIKE})")
        }
        Filter::Text(m) => fts::predicate(m, params),
        Filter::Date(range) => date_predicate(range, params),
        // `Attachments::None` is `{"kind":"none"}`; `Present` carries a count under `v`.
        Filter::HasAttachment => {
            params.push(SqlValue::Text("none".to_owned()));
            "(json_extract(ts.attachments, '$.kind') != ?)".to_owned()
        }
        // `Snooze::Until` is the only form with a wake-up instant.
        Filter::Snoozed => {
            params.push(SqlValue::Text("until".to_owned()));
            "(json_extract(ts.snooze, '$.kind') = ?)".to_owned()
        }
        Filter::SnoozeDue => snooze_due(now, params),
        // `Pin::Rank` is the pinned form; `Unpinned` has no rank.
        Filter::Pinned => {
            params.push(SqlValue::Text("rank".to_owned()));
            "(json_extract(ts.pin, '$.kind') = ?)".to_owned()
        }
    }
}

/// The threads with a message addressed in one mailbox, for [`Filter::InFolder`]. Binds the
/// account, then the path exactly as `remote_map.mailbox` holds it (decoded from modified
/// UTF-7).
const IN_FOLDER: &str = "SELECT m.thread FROM remote_map r \
     JOIN messages m ON m.id = r.message \
     WHERE r.account = ? AND r.mailbox = ?";

fn combine(
    children: &[Filter],
    op: &str,
    empty: &str,
    now: DateTime<Utc>,
    params: &mut Vec<SqlValue>,
) -> String {
    if children.is_empty() {
        return empty.to_owned();
    }
    let mut clause = String::from("(");
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            clause.push(' ');
            clause.push_str(op);
            clause.push(' ');
        }
        clause.push_str(&predicate(child, now, params));
    }
    clause.push(')');
    clause
}

/// Serde name of a role, the element `json_each` yields from a `MailboxSet` array.
fn role_name(role: MailboxRole) -> &'static str {
    match role {
        MailboxRole::Inbox => "inbox",
        MailboxRole::Archive => "archive",
        MailboxRole::Sent => "sent",
        MailboxRole::Drafts => "drafts",
        MailboxRole::Trash => "trash",
        MailboxRole::Spam => "spam",
    }
}

/// `sqlite::row::to_json` output. Fieldless enums are JSON strings, quotes included:
/// `ReadState::Read` is stored as `"read"`, not `read`.
fn stored_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("fieldless domain enum serializes to JSON")
}

/// `LIKE ? ESCAPE '\'`. The escape character is one backslash; SQLite string literals do not
/// treat `\` as an escape, so this is not doubled again in SQL.
const LIKE: &str = concat!("LIKE ? ESCAPE '", '\\', "'");

/// Match either of two columns, at least one of which may be NULL.
///
/// `coalesce` is load-bearing, not defensive. SQL is three-valued: `NULL LIKE ?` is NULL, and
/// `NULL OR false` is NULL rather than false — which still excludes the row, so the bug is
/// invisible until the clause appears under `NOT`, where `NOT NULL` is also NULL and the row
/// disappears from *both* a filter and its negation. `Filter::fit` has no third value: an
/// absent display name simply does not match. Found by the parity proptest on
/// `Not(From(Contains(..)))` against a message whose sender had no display name.
fn like_either(left: &str, right: &str, m: &TextMatch, params: &mut Vec<SqlValue>) -> String {
    let pattern = like_pattern(m);
    params.push(SqlValue::Text(pattern.clone()));
    params.push(SqlValue::Text(pattern));
    format!("(coalesce({left}, '') {LIKE} OR coalesce({right}, '') {LIKE})")
}

/// Substring or whole-value pattern. `%`, `_` and `\` in the needle are literals — a search
/// for `50%` must not match every subject.
fn like_pattern(m: &TextMatch) -> String {
    match m {
        TextMatch::Contains(needle) => format!("%{}%", escape_like(needle)),
        TextMatch::Exact(needle) => escape_like(needle),
    }
}

fn escape_like(needle: &str) -> String {
    let mut out = String::with_capacity(needle.len());
    for ch in needle.chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Must match `sqlite::row::from_time` exactly, or a comparison here is against text written
/// in a different shape. Fixed nine-digit nanoseconds: a shorter RFC 3339 form does not sort
/// against that text, because `...06.000000000Z` is less than `...06Z`.
fn summary_timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn date_predicate(range: &DateRange, params: &mut Vec<SqlValue>) -> String {
    match (range.from, range.to) {
        (None, None) => "(1=1)".to_owned(),
        (Some(from), None) => {
            params.push(SqlValue::Text(summary_timestamp(from)));
            "(ts.last_date >= ?)".to_owned()
        }
        (None, Some(to)) => {
            params.push(SqlValue::Text(summary_timestamp(to)));
            "(ts.last_date < ?)".to_owned()
        }
        (Some(from), Some(to)) => {
            params.push(SqlValue::Text(summary_timestamp(from)));
            params.push(SqlValue::Text(summary_timestamp(to)));
            "(ts.last_date >= ? AND ts.last_date < ?)".to_owned()
        }
    }
}

fn snooze_due(now: DateTime<Utc>, params: &mut Vec<SqlValue>) -> String {
    params.push(SqlValue::Text("until".to_owned()));
    // Nine fractional digits, so the bound matches the padded column text below.
    params.push(SqlValue::Text(
        now.to_rfc3339_opts(SecondsFormat::Nanos, true),
    ));
    // Chrono serde (`SecondsFormat::AutoSi`) omits fractional seconds when they are zero
    // and drops trailing zeros otherwise. Those strings do not order: `...06.001Z` is
    // less than `...06Z` as text, but the instant is later. Pad both sides to nine digits
    // and compare that text. Years 0000–9999 then sort as `DateTime` does. `rtrim` only
    // strips the `Z` suffix chrono writes (`use_z`).
    r#"(json_extract(ts.snooze, '$.kind') = ? AND (
        SELECT CASE
            WHEN instr(t, '.') = 0 THEN t || '.000000000Z'
            ELSE substr(t, 1, instr(t, '.'))
                || substr(substr(t, instr(t, '.') + 1) || '000000000', 1, 9) || 'Z'
        END
        FROM (SELECT rtrim(json_extract(ts.snooze, '$.v'), 'Z') AS t)
    ) <= ?)"#
        .to_owned()
}
