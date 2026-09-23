//! Prefix vocabulary and bm25 ranking on the SQLite store.
//!
//! The vocabulary table is TEMP, created per connection, so it is not schema and
//! `EXPECTED_VERSION` stays 8. Ranking binds [`MATCH_PREDICATE`] with the arguments
//! [`match_needles`] already builds for [`Filter::Text`].

use chrono::{DateTime, Utc};
use mail_domain::{Filter, ThreadSummary};
use rusqlite::Connection;

use super::SqliteStore;
use super::read;
use crate::prefix::{is_cjk_bigram, prefix_range};
use crate::sql::{self, MATCH_PREDICATE, MATCHING, SqlValue};
use crate::term::by_frequency;
use crate::{StoreError, Term};

const VOCAB: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS temp.messages_vocab \
     USING fts5vocab(main, messages_fts, 'row')";

/// `temp.messages_vocab` over the external-content index. A TEMP table dies with the
/// connection, so every connection creates its own.
pub(super) fn ensure_vocab(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch(VOCAB)
        .map_err(|e| StoreError::Db(e.to_string()))
}

pub(super) fn terms_with_prefix(
    db: &Connection,
    prefix: &str,
    limit: usize,
) -> Result<Vec<Term>, StoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let Some(range) = prefix_range(prefix) else {
        return Ok(Vec::new());
    };
    let mut params = vec![SqlValue::Text(range.start)];
    let sql = match range.end {
        Some(end) => {
            params.push(SqlValue::Text(end));
            "SELECT term, doc FROM temp.messages_vocab \
             WHERE term >= ? AND term < ? \
             ORDER BY doc DESC, term ASC \
             LIMIT ?"
        }
        None => {
            "SELECT term, doc FROM temp.messages_vocab \
             WHERE term >= ? \
             ORDER BY doc DESC, term ASC \
             LIMIT ?"
        }
    };
    params.push(SqlValue::Int(i64::try_from(limit).unwrap_or(i64::MAX)));

    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bound(&params)), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut terms = Vec::new();
    for row in rows {
        let (text, doc) = row?;
        let docs = u64::try_from(doc).map_err(|e| StoreError::Db(format!("vocab doc: {e}")))?;
        terms.push(Term {
            cjk_bigram: is_cjk_bigram(&text),
            text,
            docs,
        });
    }
    // The SQL orders the same way. Sorting again keeps one definition of the order.
    by_frequency(&mut terms);
    Ok(terms)
}

pub(super) fn search_ranked(
    store: &SqliteStore,
    db: &Connection,
    filter: &Filter,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Vec<(ThreadSummary, f64)>, StoreError> {
    let compiled = sql::compile(filter, now);
    let needles = sql::match_needles(filter);

    // Several text terms become one MATCH, the arguments `match_args` already builds, joined
    // with OR. `bm25` cannot be an argument of `MIN` and cannot appear in a compound SELECT,
    // so the query computes it on its own and the thread score is the minimum of that column.
    // OR scores every message that holds any of the terms; AND inside the MATCH would require
    // every word on that one row, and a thread whose words are split across messages would
    // come back unscored while `threads()` still returns it.
    //
    // When the filter uses that one MATCH clause exactly once, the CTE replaces it, so the
    // term is scanned once. Several clauses keep the compiled filter and score with a second
    // pass: an OR of the terms is not the same membership test as AND-ing each clause.
    let shared = (needles.len() == 1).then(|| rewrite_single_match(&compiled, &needles[0]));
    let mut reuse_compiled_params = true;
    let (with_clause, score_expr, join, group, mut params, where_clause) = if needles.is_empty() {
        (
            String::new(),
            "0.0".to_owned(),
            String::new(),
            String::new(),
            Vec::new(),
            compiled.where_clause,
        )
    } else if let Some((where_clause, where_params)) = shared.flatten() {
        reuse_compiled_params = false;
        let with_clause = format!(
            "WITH hits AS MATERIALIZED ( \
                SELECT m.thread AS thread, bm25(messages_fts) AS best \
                FROM messages_fts \
                JOIN messages m ON m.rowid = messages_fts.rowid \
                WHERE {MATCH_PREDICATE}) "
        );
        (
            with_clause,
            "COALESCE(-MIN(hits.best), 0.0)".to_owned(),
            "LEFT JOIN hits ON hits.thread = ts.thread".to_owned(),
            "GROUP BY ts.thread".to_owned(),
            std::iter::once(SqlValue::Text(needles[0].clone()))
                .chain(where_params)
                .collect(),
            where_clause,
        )
    } else {
        let join = format!(
            "LEFT JOIN ( \
                SELECT m.thread AS thread, bm25(messages_fts) AS best \
                FROM messages_fts \
                JOIN messages m ON m.rowid = messages_fts.rowid \
                WHERE {MATCH_PREDICATE} \
             ) score ON score.thread = ts.thread"
        );
        (
            String::new(),
            "COALESCE(-MIN(score.best), 0.0)".to_owned(),
            join,
            "GROUP BY ts.thread".to_owned(),
            vec![SqlValue::Text(needles.join(" OR "))],
            compiled.where_clause,
        )
    };

    let (columns, score_at) = qualified_columns();
    let sql_text = format!(
        "{with_clause}SELECT {columns}, {score_expr} AS relevance \
         FROM thread_summary ts \
         {join} \
         WHERE {where_clause} \
         {group} \
         ORDER BY relevance DESC, ts.last_date DESC \
         LIMIT ?"
    );
    if reuse_compiled_params {
        params.extend(compiled.params);
    }
    params.push(SqlValue::Int(i64::try_from(limit).unwrap_or(i64::MAX)));

    let mut stmt = db.prepare(&sql_text)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(bound(&params)))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let summary = store.read_summary(row)?;
        let score: f64 = row.get(score_at)?;
        out.push((summary, score));
    }
    Ok(out)
}

/// The best `k` of the `window` most recent threads `filter` matches, by full-text relevance.
///
/// What costs is `bm25`, and `search_ranked` computes it for every message a term hits — every
/// message, for a word like `the`. Here the window is chosen first, by the same indexed
/// date-ordered query [`Store::threads`](crate::Store::threads) runs, and only the messages of
/// those threads are scored: each one probed in the index by rowid, so the work is bounded by
/// the window and not by how common the word is.
///
/// A thread in the window with no message hitting a text term — one kept by a non-text branch
/// of an `Or` — has no relevance, and is not a top hit.
pub(super) fn top_hits(
    store: &SqliteStore,
    db: &Connection,
    filter: &Filter,
    k: usize,
    window: usize,
    now: DateTime<Utc>,
) -> Result<Vec<(ThreadSummary, f64)>, StoreError> {
    let needles = sql::match_needles(filter);
    if needles.is_empty() || k == 0 || window == 0 {
        return Ok(Vec::new());
    }
    let compiled = sql::compile(filter, now);
    let (columns, score_at) = qualified_columns();
    let where_clause = &compiled.where_clause;
    // The MATCH drives, walking the term's doclist, and the window is a filter on it. The
    // unary `+` keeps that filter away from FTS5's planner: offered a rowid constraint as well,
    // FTS5 picks a rowid lookup, which is not a full-text query and has no `bm25`. Walking a
    // doclist is cheap even for `the`; what costs is scoring, and `bm25` is evaluated only for
    // the rows the filter keeps — the messages of the window.
    let sql_text = format!(
        "WITH recent AS MATERIALIZED ( \
            SELECT ts.thread AS thread FROM thread_summary ts \
            WHERE {where_clause} \
            ORDER BY ts.last_date DESC LIMIT ?), \
         candidates AS MATERIALIZED ( \
            SELECT m.rowid AS rowid, m.thread AS thread \
            FROM recent JOIN messages m ON m.thread = recent.thread), \
         scored AS MATERIALIZED ( \
            SELECT candidates.thread AS thread, bm25(messages_fts) AS s \
            FROM messages_fts JOIN candidates ON candidates.rowid = +messages_fts.rowid \
            WHERE {MATCH_PREDICATE}) \
         SELECT {columns}, -MIN(scored.s) AS relevance \
         FROM scored JOIN thread_summary ts ON ts.thread = scored.thread \
         GROUP BY ts.thread \
         ORDER BY relevance DESC, ts.last_date DESC \
         LIMIT ?"
    );
    let mut params = compiled.params;
    params.push(SqlValue::Int(i64::try_from(window).unwrap_or(i64::MAX)));
    params.push(SqlValue::Text(needles.join(" OR ")));
    params.push(SqlValue::Int(i64::try_from(k).unwrap_or(i64::MAX)));

    let mut stmt = db.prepare(&sql_text)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(bound(&params)))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let summary = store.read_summary(row)?;
        let score: f64 = row.get(score_at)?;
        out.push((summary, score));
    }
    Ok(out)
}

/// The summary columns, each qualified with `ts.`, and the index of the column after them.
///
/// Column by column, not by text replacement: a later column whose name ends in `thread` must
/// not be rewritten along with `thread`.
fn qualified_columns() -> (String, usize) {
    let qualified: Vec<String> = read::SUMMARY_COLUMNS
        .split(',')
        .map(|column| format!("ts.{}", column.trim()))
        .collect();
    (qualified.join(", "), qualified.len())
}

/// Replace the one thread-membership MATCH in `compiled` with membership in the `hits` CTE.
///
/// The placeholder that belonged to that MATCH is dropped; the caller binds the same text as
/// the CTE's own parameter. `None` when the clause isn't exactly one occurrence, or the bound
/// text isn't `needle` — then the caller keeps the compiled filter untouched.
///
/// The parameter is found by counting `?` before the clause, which assumes the compiled WHERE
/// has no literal `?` of its own. `sql::compile` binds every value today; if it ever writes a
/// string literal containing `?`, the count is off by one. The check that the bound text
/// equals `needle` then fails and this returns `None`, so the result is slower, never wrong.
fn rewrite_single_match(
    compiled: &sql::SqlFilter,
    needle: &str,
) -> Option<(String, Vec<SqlValue>)> {
    let mut found = compiled.where_clause.match_indices(MATCHING);
    let (at, _) = found.next()?;
    if found.next().is_some() {
        return None;
    }
    let before = compiled.where_clause[..at].matches('?').count();
    match compiled.params.get(before) {
        Some(SqlValue::Text(text)) if text == needle => {}
        _ => return None,
    }
    let where_clause =
        compiled
            .where_clause
            .replacen(MATCHING, "ts.thread IN (SELECT thread FROM hits)", 1);
    let mut params = compiled.params.clone();
    params.remove(before);
    Some((where_clause, params))
}

fn bound(params: &[SqlValue]) -> Vec<rusqlite::types::Value> {
    params
        .iter()
        .map(|value| match value {
            SqlValue::Text(text) => rusqlite::types::Value::Text(text.clone()),
            SqlValue::Int(n) => rusqlite::types::Value::Integer(*n),
        })
        .collect()
}
