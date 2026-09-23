//! Folding a typed prefix into the token the index stored, and the range that finds it.
//!
//! The vocabulary holds `search_tokens` output. A prefix matched as typed misses `resume` when
//! the user typed `Rés`, and a `LIKE` pattern treats `%` and `_` as wildcards.

use mail_domain::filter::search_tokens;

/// The half-open range of indexed terms that begin with the folded prefix.
pub(crate) struct PrefixRange {
    pub start: String,
    /// Exclusive upper bound. `None` only when `start` is a string of U+10FFFF, which has no
    /// successor: every term at or after `start` qualifies.
    pub end: Option<String>,
}

/// Fold `prefix` the way the indexer did, if that produces exactly one token.
///
/// Empty text and punctuation fold to nothing, and that is an empty suggestion list rather
/// than the whole vocabulary. Several tokens are several complete terms, not one prefix — the
/// caller passes the single word being typed.
pub(crate) fn folded_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return None;
    }
    let mut tokens = search_tokens(prefix);
    if tokens.len() == 1 {
        tokens.pop()
    } else {
        None
    }
}

/// `term >= start AND term < end` in SQLite's BINARY collation.
///
/// UTF-8 byte order and Unicode scalar order agree, so the successor of the last scalar is the
/// successor of the prefix. `%` and `_` never become wildcards: they are either folded away
/// (they are not token characters) or, if a prefix still contained one, compared as bytes.
pub(crate) fn prefix_range(prefix: &str) -> Option<PrefixRange> {
    let start = folded_prefix(prefix)?;
    let end = prefix_successor(&start);
    Some(PrefixRange { start, end })
}

/// Whether `term` falls inside [`prefix_range`]. The memory store matches with this so it
/// answers the same question as the SQL range.
pub(crate) fn term_has_prefix(term: &str, prefix: &str) -> bool {
    let Some(range) = prefix_range(prefix) else {
        return false;
    };
    match range.end {
        Some(end) => term >= range.start.as_str() && term < end.as_str(),
        None => term >= range.start.as_str(),
    }
}

fn prefix_successor(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = next_scalar(last) {
            chars.push(next);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

fn next_scalar(c: char) -> Option<char> {
    let mut u = u32::from(c).checked_add(1)?;
    // Surrogate code points are not scalars. The next scalar after U+D7FF is U+E000.
    if (0xD800..=0xDFFF).contains(&u) {
        u = 0xE000;
    }
    char::from_u32(u)
}

/// Two characters of the script the indexer bigrams: CJK ideographs and kana.
///
/// Those ranges are `is_scriptio_continua` in `mail-domain`, which is private, and that crate
/// is frozen. They are repeated here so a stored bigram can be told from a word.
pub(crate) fn is_cjk_bigram(term: &str) -> bool {
    let mut chars = term.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(a), Some(b), None) => scriptio_continua(a) && scriptio_continua(b),
        _ => false,
    }
}

fn scriptio_continua(ch: char) -> bool {
    matches!(
        ch,
        '\u{3040}'..='\u{30ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `zz{` is inside the prefix `zz` — `{` is the character after `z`, and appending it is
    /// still a string the prefix owns. The upper bound is `z{`, which does not start with `zz`.
    const RANGE: &[(&str, &str, bool)] = &[
        ("zz", "zz", true),
        ("zz", "zza", true),
        ("zz", "zz{", true),
        ("zz", "z{", false),
        ("zz", "za", false),
        ("zz", "zzz", true),
        ("%", "resume", false),
        ("%", "%", false),
        ("_", "a", false),
        ("_", "_", false),
        ("_", "_x", false),
        ("Rés", "resume", true),
        ("RES", "resume", true),
        ("res", "resume", true),
        ("", "resume", false),
        ("電", "電子", true),
        ("電", "子郵", false),
        ("電", "郵件", false),
    ];

    #[test]
    fn prefix_range_table() {
        for (prefix, term, expect) in RANGE {
            assert_eq!(
                term_has_prefix(term, prefix),
                *expect,
                "prefix {prefix:?} against {term:?}"
            );
        }
    }

    #[test]
    fn folding_is_one_token() {
        assert_eq!(folded_prefix("Rés").as_deref(), Some("res"));
        assert_eq!(folded_prefix("résumé").as_deref(), Some("resume"));
        assert_eq!(folded_prefix("RES").as_deref(), Some("res"));
        assert_eq!(folded_prefix(""), None);
        assert_eq!(folded_prefix("%"), None);
        assert_eq!(folded_prefix("_"), None);
        assert_eq!(folded_prefix("zz{").as_deref(), Some("zz"));
        assert_eq!(folded_prefix("電").as_deref(), Some("電"));
        assert_eq!(folded_prefix("電子").as_deref(), Some("電子"));
        // Three ideographs are two overlapping bigrams, not one prefix.
        assert_eq!(folded_prefix("電子郵件"), None);
    }

    #[test]
    fn bigram_flag_is_two_scriptio_continua_characters() {
        assert!(is_cjk_bigram("電子"));
        assert!(is_cjk_bigram("あい"));
        assert!(!is_cjk_bigram("電"));
        assert!(!is_cjk_bigram("resume"));
        assert!(!is_cjk_bigram("電子子"));
    }
}
