//! Prefix vocabulary and bm25 ranking on the SQLite store.
//!
//! The vocabulary table is TEMP, created per connection, so it is not schema and
//! `EXPECTED_VERSION` is unaffected. Ranking binds [`MATCH_PREDICATE`] with the arguments
//! [`match_needles`] already builds for [`Filter::Text`].

use mail_domain::{Filter, ThreadId, ThreadSummary};
use rusqlite::Connection;

use super::SqliteStore;
use super::read;
use crate::prefix::{is_cjk_bigram, prefix_range};
use crate::sql::{self, MATCH_PREDICATE, SqlValue};
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

/// The best `k` of the threads in `window`, by full-text relevance to `filter`'s text terms.
///
/// The window is the caller's: the threads it has already listed, newest first, by the same
/// indexed query [`Store::threads`](crate::Store::threads) runs. Choosing it here again meant
/// evaluating the filter's full-text membership a second time — every thread a common word
/// hits, materialised before any `LIMIT` — so a strip over the first 200 cost as much as the
/// list did. Here only the window's messages are scored, and the work is bounded by the window
/// and not by how common the word is.
///
/// Only the filter's text terms are read; the rest of it already chose the window. A thread in
/// the window with no message hitting a text term — one kept by a non-text branch of an `Or` —
/// has no relevance, and is not a top hit.
pub(super) fn top_hits(
    store: &SqliteStore,
    db: &Connection,
    filter: &Filter,
    k: usize,
    window: &[ThreadId],
) -> Result<Vec<(ThreadSummary, f64)>, StoreError> {
    let needles = sql::match_needles(filter);
    if needles.is_empty() || k == 0 || window.is_empty() {
        return Ok(Vec::new());
    }
    let (columns, score_at) = qualified_columns();
    // The MATCH drives, walking the term's doclist, and the window is a filter on it. The
    // unary `+` keeps that filter away from FTS5's planner: offered a rowid constraint as well,
    // FTS5 picks a rowid lookup, which is not a full-text query and has no `bm25`. Walking a
    // doclist is cheap even for `the`; what costs is scoring, and `bm25` is evaluated only for
    // the rows the filter keeps — the messages of the window.
    let sql_text = format!(
        "WITH candidates AS MATERIALIZED ( \
            SELECT m.rowid AS rowid, m.thread AS thread FROM messages m \
            WHERE m.thread IN (SELECT value FROM json_each(?))), \
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
    let ids: Vec<String> = window.iter().map(|id| id.to_string()).collect();
    let ids = serde_json::to_string(&ids).map_err(|e| StoreError::Db(format!("window: {e}")))?;
    let params = [
        SqlValue::Text(ids),
        SqlValue::Text(needles.join(" OR ")),
        SqlValue::Int(i64::try_from(k).unwrap_or(i64::MAX)),
    ];

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

fn bound(params: &[SqlValue]) -> Vec<rusqlite::types::Value> {
    params
        .iter()
        .map(|value| match value {
            SqlValue::Text(text) => rusqlite::types::Value::Text(text.clone()),
            SqlValue::Int(n) => rusqlite::types::Value::Integer(*n),
        })
        .collect()
}
