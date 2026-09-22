//! The full-text half of the compiler: what [`Filter::Text`] indexes, and how it asks.
//!
//! Both halves go through [`search_tokens`], the same function `Filter::fit` uses, so the index
//! holds exactly the tokens a query will ask for and the two sides of the parity test are one
//! function rather than two that agree. `unicode61` still runs over the indexed text and the
//! query; `tests/fold_table.rs` proves it leaves every token as it found it.
//!
//! Tokenizing here rather than in FTS5 is also what makes Chinese searchable at all: `unicode61`
//! makes one token of an unbroken run of ideographs, so a subject was one word and only typing
//! all of it found anything (FINDINGS F125).
//!
//! [`Filter::Text`]: mail_domain::Filter::Text

use super::SqlValue;
use mail_domain::filter::search_tokens;
use mail_domain::{Address, TextMatch};

/// The indexed text of one message: every field [`Filter::Text`] searches, as tokens.
///
/// Three writers need this — insert, body arrival, and the backfill — and they used to list the
/// fields separately, which is how a field gets added to two of them. `To` and `Cc` are here
/// because a thread is found by who it was addressed to as well as by who wrote it; `Bcc` is
/// not, for the reason `ThreadSummary::recipients` gives.
///
/// [`Filter::Text`]: mail_domain::Filter::Text
pub(crate) fn message_index(
    subject: &str,
    from: &Address,
    to: &[Address],
    cc: &[Address],
    body: Option<&str>,
) -> String {
    let mut fields = vec![
        Some(subject),
        from.name.as_deref(),
        Some(from.email.as_str()),
    ];
    for addr in to.iter().chain(cc) {
        fields.push(addr.name.as_deref());
        fields.push(Some(addr.email.as_str()));
    }
    fields.push(body);
    fields
        .into_iter()
        .flatten()
        .flat_map(search_tokens)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The SQL for one [`Filter::Text`] clause.
///
/// [`Filter::Text`]: mail_domain::Filter::Text
pub(super) fn predicate(m: &TextMatch, params: &mut Vec<SqlValue>) -> String {
    let raw = match m {
        TextMatch::Contains(needle) | TextMatch::Exact(needle) => needle,
    };
    let tokens = search_tokens(raw);
    // An empty MATCH is a syntax error in FTS5, and `fit` agrees: no tokens, no match.
    if tokens.is_empty() {
        return "(1=0)".to_owned();
    }
    // `messages_fts MATCH`, not `f MATCH`: with the table aliased, SQLite resolves MATCH
    // against the real table name and reports the alias as "no such column".
    //
    // **Uncorrelated on purpose.** This was `EXISTS (... WHERE m.thread = ts.thread AND
    // messages_fts MATCH ?)`, which mentions the outer row and so runs once *per thread*: the
    // FTS index was used, ten thousand times over, and a search of a ten-thousand-message
    // mailbox took four and a half seconds. Without the correlation SQLite evaluates the match
    // once, materialises the threads it hit, and probes that — the difference between a search
    // that is linear in the mailbox and one that is quadratic.
    const MATCHING: &str = "ts.thread IN (SELECT m.thread FROM messages_fts f \
        JOIN messages m ON m.rowid = f.rowid \
        WHERE messages_fts MATCH ?)";

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
                params.push(SqlValue::Text(quoted(token)));
                clauses.push(MATCHING.to_owned());
            }
            format!("({})", clauses.join(" AND "))
        }
        // A phrase must be adjacent and in order, which only means anything within one
        // message's indexed text, so this stays a single MATCH.
        TextMatch::Exact(_) => {
            params.push(SqlValue::Text(quoted(&tokens.join(" "))));
            format!("({MATCHING})")
        }
    }
}
/// Tokens as one FTS5 string: `"t1 t2"`, a phrase, or `"t1"`, a single term.
///
/// Quoted so `OR`, `NEAR`, `*`, `^` and `-` are data. Search tokens never contain a quote, since
/// `"` separates words, but an internal one is doubled anyway: that is FTS5's escape, and this
/// is the last line of defence between a needle and the query syntax.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}
