//! `Filter::fit` and the FTS5 index agree on every code point, not only on the ones a proptest
//! happens to generate.
//!
//! Both sides tokenize with `mail_domain::filter::search_tokens`: the store indexes its output
//! and queries with its output. SQLite's `unicode61 remove_diacritics 2` tokenizer still runs
//! over that, so parity comes down to one property — SQLite changes nothing it is handed. That
//! holds for every string exactly when it holds for every character a token can contain, and
//! there are only 1.1 million of those, so this asks about all of them.
//!
//! The same census regenerates `mail-domain/src/filter/fold.rs`, the table of code points where
//! SQLite departs from Rust's lowercase, and fails when the checked-in table has drifted from it:
//!
//! ```text
//! cargo test -p mail-store --test fold_table -- --ignored regenerate
//! ```

use mail_domain::filter::search_tokens;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;

/// What SQLite does with one code point in the middle of a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fold {
    Keep,
    To(char),
    Drop,
    Split,
}

/// Every code point that can appear in text, in order, with what SQLite does to it.
fn census() -> &'static [(char, Fold)] {
    static CENSUS: OnceLock<Vec<(char, Fold)>> = OnceLock::new();
    CENSUS.get_or_init(ask_sqlite)
}

/// Index each code point as `q<c>q` and read back what the tokenizer made of it.
///
/// `q` on both sides tells the four cases apart: `q`, `q` is a separator; `qq` is a character
/// that joins the word and contributes nothing; `q<x>q` is one that joins it as `x`. Batched,
/// many to a row, because a row per code point is a million inserts; the vocabulary table lists
/// each row's terms in order, which is all that is needed to walk them back.
fn ask_sqlite() -> Vec<(char, Fold)> {
    const PER_ROW: usize = 4096;
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(x, tokenize = 'unicode61 remove_diacritics 2');
         CREATE VIRTUAL TABLE terms USING fts5vocab(t, 'instance');",
    )
    .unwrap();

    // NUL is the one character a text column cannot carry through FTS5; nothing else is skipped.
    let chars: Vec<char> = ('\u{1}'..=char::MAX).collect();
    for (row, chunk) in chars.chunks(PER_ROW).enumerate() {
        let text: String = chunk.iter().map(|c| format!("q{c}q ")).collect();
        db.execute(
            "INSERT INTO t (rowid, x) VALUES (?1, ?2)",
            rusqlite::params![row as i64, text],
        )
        .unwrap();
    }

    let mut stmt = db
        .prepare("SELECT doc, term FROM terms ORDER BY doc, offset")
        .unwrap();
    let mut terms: Vec<Vec<String>> = vec![Vec::new(); chars.len().div_ceil(PER_ROW)];
    for row in stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)? as usize, r.get::<_, String>(1)?))
        })
        .unwrap()
    {
        let (doc, term) = row.unwrap();
        terms[doc].push(term);
    }

    let mut out = Vec::with_capacity(chars.len());
    for (chunk, terms) in chars.chunks(PER_ROW).zip(terms) {
        let mut terms = terms.into_iter();
        for &c in chunk {
            let term = terms.next().unwrap();
            let inner: Vec<char> = term.chars().collect();
            let fold = match inner.as_slice() {
                ['q'] => {
                    assert_eq!(terms.next().as_deref(), Some("q"), "U+{:04X}", u32::from(c));
                    Fold::Split
                }
                ['q', 'q'] => Fold::Drop,
                ['q', x, 'q'] if *x == c => Fold::Keep,
                ['q', x, 'q'] => Fold::To(*x),
                _ => panic!("U+{:04X} became {term:?}", u32::from(c)),
            };
            out.push((c, fold));
        }
        assert!(terms.next().is_none());
    }
    out
}

/// What `search_tokens` does with a code point the table does not list.
fn rust_default(c: char) -> Fold {
    match c.to_lowercase().next() {
        Some(lower) if lower.is_alphanumeric() && lower == c => Fold::Keep,
        Some(lower) if lower.is_alphanumeric() => Fold::To(lower),
        _ => Fold::Split,
    }
}

/// The source of `fold.rs`: every departure from [`rust_default`], identical neighbours merged.
fn generate() -> String {
    let mut entries: Vec<(u32, u32, Fold)> = Vec::new();
    for &(c, fold) in census() {
        // ASCII never reaches the table: `search_tokens` answers it before looking.
        if c.is_ascii() || fold == rust_default(c) {
            continue;
        }
        let cp = u32::from(c);
        match entries.last_mut() {
            Some((_, hi, last))
                if *hi + 1 == cp && *last == fold && !matches!(fold, Fold::To(_)) =>
            {
                *hi = cp;
            }
            _ => entries.push((cp, cp, fold)),
        }
    }

    let mut out = String::from(
        "//! Where SQLite's `unicode61 remove_diacritics 2` tokenizer departs from Rust's lowercase.
//!
//! @generated by `cargo test -p mail-store --test fold_table -- --ignored regenerate`, from the
//! SQLite that `mail-store` bundles. Do not edit: that crate's `the_fold_table_is_what_sqlite_does`
//! fails on any difference, including one made by upgrading SQLite or Rust underneath it.

use super::tokens::Fold;

/// `(first, last, fold)` over code points: sorted, disjoint, and silent about ASCII.
pub(super) const TABLE: &[(u32, u32, Fold)] = &[
",
    );
    for (lo, hi, fold) in entries {
        let fold = match fold {
            Fold::Keep => "Fold::Keep".to_owned(),
            Fold::To(x) => format!("Fold::To('\\u{{{:x}}}')", u32::from(x)),
            Fold::Drop => "Fold::Drop".to_owned(),
            Fold::Split => "Fold::Split".to_owned(),
        };
        writeln!(out, "    (0x{lo:04x}, 0x{hi:04x}, {fold}),").unwrap();
    }
    out.push_str("];\n");
    out
}

fn table_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../mail-domain/src/filter/fold.rs")
}

/// The parity property itself: every character a token can hold is one SQLite keeps as it is.
#[test]
fn sqlite_changes_no_search_token() {
    let keeps = |ch: char| {
        census()
            .binary_search_by_key(&ch, |&(c, _)| c)
            .is_ok_and(|i| census()[i].1 == Fold::Keep)
    };
    let mut wrong = Vec::new();
    for &(c, _) in census() {
        for token in search_tokens(&format!("q{c}q")) {
            if let Some(bad) = token.chars().find(|&ch| !keeps(ch)) {
                wrong.push(format!(
                    "U+{:04X} gives {token:?}, and SQLite alters {bad:?}",
                    u32::from(c)
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} code points, first: {:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(10)]
    );
}

#[test]
fn the_fold_table_is_what_sqlite_does() {
    let checked_in = std::fs::read_to_string(table_path()).unwrap();
    assert!(
        checked_in == generate(),
        "fold.rs no longer matches this SQLite. Regenerate it:\n\
         cargo test -p mail-store --test fold_table -- --ignored regenerate"
    );
}

#[test]
#[ignore = "writes mail-domain/src/filter/fold.rs"]
fn regenerate() {
    std::fs::write(table_path(), generate()).unwrap();
}
