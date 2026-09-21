//! Compiling a [`Filter`] into SQL.
//!
//! This module and [`mail_domain::Filter::fit`] are **two implementations of one semantics**.
//! They will diverge, silently, and the symptom is "search quietly missed a message". The
//! defence is `tests/parity.rs`, which asserts `fit(f, ctx) == (id in sql(f))` over generated
//! filters and corpora. If that test is ever skipped or weakened, the bug ships.

use chrono::{DateTime, SecondsFormat, Utc};
use mail_domain::{DateRange, Filter, MailboxRole, TextMatch};
use serde::Serialize;

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
        Filter::Text(m) => text_predicate(m, params),
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

fn like_either(left: &str, right: &str, m: &TextMatch, params: &mut Vec<SqlValue>) -> String {
    let pattern = like_pattern(m);
    params.push(SqlValue::Text(pattern.clone()));
    params.push(SqlValue::Text(pattern));
    format!("({left} {LIKE} OR {right} {LIKE})")
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

fn text_predicate(m: &TextMatch, params: &mut Vec<SqlValue>) -> String {
    let raw = match m {
        TextMatch::Contains(needle) | TextMatch::Exact(needle) => needle,
    };
    let tokens = fts_tokens(raw);
    // An empty MATCH is a syntax error in FTS5, and `fit` agrees: no tokens, no match.
    if tokens.is_empty() {
        return "(1=0)".to_owned();
    }
    // `messages_fts MATCH`, not `f MATCH`: with the table aliased, SQLite resolves MATCH
    // against the real table name and reports the alias as "no such column".
    const EXISTS: &str = "EXISTS (SELECT 1 FROM messages m \
        JOIN messages_fts f ON f.rowid = m.rowid \
        WHERE m.thread = ts.thread AND messages_fts MATCH ?)";

    match m {
        // One EXISTS per token, ANDed at the SQL level rather than inside one MATCH.
        //
        // `t1 AND t2` inside a single MATCH requires both tokens on the SAME message row, but
        // `Filter::fit` treats the whole thread as one corpus: a conversation with "ada" in
        // the first message and "lunch" in the reply matches `fit`. Splitting the tokens into
        // separate correlated EXISTS asks "does each token appear SOMEWHERE in this thread",
        // which is the same question `fit` answers.
        TextMatch::Contains(_) => {
            let mut clauses = Vec::with_capacity(tokens.len());
            for token in &tokens {
                params.push(SqlValue::Text(quote_fts_token(token)));
                clauses.push(EXISTS.to_owned());
            }
            format!("({})", clauses.join(" AND "))
        }
        // A phrase must be adjacent and in order, which only means anything within one
        // message's indexed text, so this stays a single MATCH.
        TextMatch::Exact(_) => {
            params.push(SqlValue::Text(fts_phrase(&tokens)));
            format!("({EXISTS})")
        }
    }
}

/// Alphanumeric runs, with combining marks dropped rather than used as separators.
///
/// That is how `unicode61 remove_diacritics 2` tokenizes a decomposed letter (`e` + U+0301
/// stays one token) and how [`mail_domain::Filter`]'s full-text matcher splits the needle.
/// Case and diacritics are left for FTS5: the same tokenizer folds the query and the index.
fn fts_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if is_combining_mark(ch) {
            continue;
        }
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The mark ranges `Filter::fit` ignores when it tokenizes. Kept in step with that function
/// so a decomposed letter is one token on both sides.
fn is_combining_mark(ch: char) -> bool {
    matches!(
        ch,
        '\u{0300}'..='\u{036f}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{20d0}'..='\u{20f0}'
            | '\u{fe20}'..='\u{fe2f}'
    )
}

/// One FTS5 token, quoted, so `OR`, `NEAR`, `*`, `^` and `-` in the needle are data.
/// An internal `"` is doubled, which is how FTS5 escapes a quote inside a phrase.
fn quote_fts_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len() + 2);
    out.push('"');
    for ch in token.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// A phrase: the tokens, in order, inside one pair of quotes. `"t1 t2"`, not `"t1" AND "t2"`.
fn fts_phrase(tokens: &[String]) -> String {
    let mut out = String::from("\"");
    for (i, token) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for ch in token.chars() {
            if ch == '"' {
                out.push('"');
            }
            out.push(ch);
        }
    }
    out.push('"');
    out
}
