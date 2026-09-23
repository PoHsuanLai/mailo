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
use mail_domain::{Address, Filter, TextMatch};

/// `messages_fts MATCH ?`. Ranking binds the same predicate this module builds for
/// [`Filter::Text`]; the table name is required, because with the table aliased SQLite resolves
/// MATCH against the real name and reports the alias as "no such column".
///
/// [`Filter::Text`]: mail_domain::Filter::Text
pub(crate) const MATCH_PREDICATE: &str = "messages_fts MATCH ?";

/// The thread-membership clause [`predicate`] emits for one MATCH argument.
///
/// [`predicate`]: predicate
pub(crate) const MATCHING: &str = concat!(
    "ts.thread IN (SELECT m.thread FROM messages_fts f \
    JOIN messages m ON m.rowid = f.rowid \
    WHERE ",
    "messages_fts MATCH ?)"
);

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
    let args = match_args(m);
    // An empty MATCH is a syntax error in FTS5, and `fit` agrees: no tokens, no match.
    if args.is_empty() {
        return "(1=0)".to_owned();
    }
    // **Uncorrelated on purpose.** This was `EXISTS (... WHERE m.thread = ts.thread AND
    // messages_fts MATCH ?)`, which mentions the outer row and so runs once *per thread*: the
    // FTS index was used, ten thousand times over, and a search of a ten-thousand-message
    // mailbox took four and a half seconds. Without the correlation SQLite evaluates the match
    // once, materialises the threads it hit, and probes that — the difference between a search
    // that is linear in the mailbox and one that is quadratic.
    //
    // One MATCH per argument, ANDed at the SQL level rather than inside one MATCH.
    //
    // `t1 AND t2` inside a single MATCH requires both tokens on the SAME message row, but
    // `Filter::fit` treats the whole thread as one corpus: a conversation with "ada" in
    // the first message and "lunch" in the reply matches `fit`. One argument per token of a
    // [`TextMatch::Contains`] asks "does each token appear SOMEWHERE in this thread". A
    // phrase ([`TextMatch::Exact`]) is one argument, because adjacency only means anything
    // within one message.
    //
    // [`TextMatch::Contains`]: mail_domain::TextMatch::Contains
    // [`TextMatch::Exact`]: mail_domain::TextMatch::Exact
    let mut clauses = Vec::with_capacity(args.len());
    for arg in args {
        params.push(SqlValue::Text(arg));
        clauses.push(MATCHING.to_owned());
    }
    format!("({})", clauses.join(" AND "))
}

/// The `messages_fts MATCH` arguments for one [`TextMatch`], in the form [`predicate`] binds.
///
/// Empty when the needle folds to no tokens. [`TextMatch::Contains`] is one quoted term per
/// token. [`TextMatch::Exact`] is one argument, the tokens as a phrase.
///
/// [`TextMatch`]: mail_domain::TextMatch
/// [`TextMatch::Contains`]: mail_domain::TextMatch::Contains
/// [`TextMatch::Exact`]: mail_domain::TextMatch::Exact
pub(crate) fn match_args(m: &TextMatch) -> Vec<String> {
    let raw = match m {
        TextMatch::Contains(needle) | TextMatch::Exact(needle) => needle,
    };
    let tokens = search_tokens(raw);
    if tokens.is_empty() {
        return Vec::new();
    }
    match m {
        TextMatch::Contains(_) => tokens.iter().map(|token| quoted(token)).collect(),
        TextMatch::Exact(_) => vec![quoted(&tokens.join(" "))],
    }
}

/// Every positive [`Filter::Text`] argument in `filter`.
///
/// Negated text is omitted: a thread a `Not` keeps did not match that term, so it has no
/// `bm25` for it. The strings are [`match_args`], which is what [`predicate`] binds.
///
/// [`Filter::Text`]: mail_domain::Filter::Text
pub(crate) fn match_needles(filter: &Filter) -> Vec<String> {
    let mut out = Vec::new();
    collect_needles(filter, false, &mut out);
    out
}

fn collect_needles(filter: &Filter, negated: bool, out: &mut Vec<String>) {
    match filter {
        Filter::And(children) | Filter::Or(children) => {
            for child in children {
                collect_needles(child, negated, out);
            }
        }
        Filter::Not(inner) => collect_needles(inner, !negated, out),
        Filter::Text(m) if !negated => {
            for arg in match_args(m) {
                if !out.contains(&arg) {
                    out.push(arg);
                }
            }
        }
        _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thread_clause_uses_the_shared_match_predicate() {
        assert!(
            MATCHING.contains(MATCH_PREDICATE),
            "ranking and filtering must share one MATCH spelling"
        );
        let args = match_args(&TextMatch::Contains("Ada Lunch".into()));
        assert_eq!(args, vec!["\"ada\"".to_owned(), "\"lunch\"".to_owned()]);
        let phrase = match_args(&TextMatch::Exact("Ada Lunch".into()));
        assert_eq!(phrase, vec!["\"ada lunch\"".to_owned()]);
    }
}
