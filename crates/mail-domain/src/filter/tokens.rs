//! The full-text tokenizer: the one function both halves of [`Filter::Text`] run.
//!
//! The store indexes exactly what this returns and queries with exactly what this returns, and
//! [`Filter::fit`] compares exactly what this returns. SQLite's `unicode61 remove_diacritics 2`
//! tokenizer still sits under the index, so the one property left to hold is that it leaves this
//! function's output alone — every character in a token is one SQLite keeps as itself. The
//! store's `tests/fold_table.rs` checks that for every code point in Unicode.
//!
//! Folding follows SQLite's own table, not Rust's lowercase, so search finds what it always
//! found: `Résumé` is `resume`, `ς` is `σ`, `ſ` is `s`. [`fold`](super::fold) holds the code
//! points where the two disagree, generated from SQLite and never edited by hand.
//!
//! [`Filter::Text`]: super::Filter::Text
//! [`Filter::fit`]: super::Filter::fit

use super::fold::TABLE;

/// What one code point contributes to a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fold {
    /// Part of a word, as itself.
    Keep,
    /// Part of a word, as this character instead.
    To(char),
    /// Part of a word, contributing nothing: a combining accent, so `e` + U+0301 is `e`.
    Drop,
    /// Not part of a word.
    Split,
}

/// Split `text` into search tokens. `"Résumé, v2 (final)"` is `["resume", "v2", "final"]`.
pub fn search_tokens(text: &str) -> Vec<String> {
    let mut tokens = Tokens::default();
    for ch in text.chars() {
        match fold(ch) {
            Fold::Drop => {}
            Fold::Split => tokens.end(),
            Fold::Keep => tokens.push(ch),
            Fold::To(folded) => tokens.push(folded),
        }
    }
    tokens.end();
    tokens.out
}

/// How SQLite treats `ch` inside a word: the generated table where it says so, otherwise Rust's
/// lowercase — which is what SQLite does for every code point the table does not list.
fn fold(ch: char) -> Fold {
    if ch.is_ascii() {
        return if ch.is_ascii_alphanumeric() {
            Fold::To(ch.to_ascii_lowercase())
        } else {
            Fold::Split
        };
    }
    let cp = u32::from(ch);
    let listed = TABLE.binary_search_by(|&(lo, hi, _)| {
        if hi < cp {
            std::cmp::Ordering::Less
        } else if lo > cp {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    if let Ok(i) = listed {
        return TABLE[i].2;
    }
    match ch.to_lowercase().next() {
        Some(lower) if lower.is_alphanumeric() => Fold::To(lower),
        _ => Fold::Split,
    }
}

/// Tokens being built: the word in progress, and a run of ideographs beside it.
#[derive(Default)]
struct Tokens {
    out: Vec<String>,
    word: String,
    /// Ideographs or kana, held apart from `word`: a run ends at the first character that is not
    /// one, including an ordinary letter, so `NTU臺大` is `ntu` and then the Chinese run.
    run: Vec<char>,
}

impl Tokens {
    fn push(&mut self, ch: char) {
        if is_scriptio_continua(ch) {
            self.end_word();
            self.run.push(ch);
        } else {
            self.end_run();
            self.word.push(ch);
        }
    }

    fn end(&mut self) {
        self.end_word();
        self.end_run();
    }

    fn end_word(&mut self) {
        if !self.word.is_empty() {
            self.out.push(std::mem::take(&mut self.word));
        }
    }

    /// A run contributes overlapping bigrams: `臺大計中` is `臺大`, `大計`, `計中`.
    ///
    /// Bigrams rather than single characters because a one-character query is almost never what
    /// anyone means in Chinese, and separate characters ANDed would match any message with 臺 and
    /// 大 anywhere in it. Overlapping, so a needle starting mid-word still matches: `大計` is a
    /// token of `臺大計中`. A run of one is that character, so an isolated ideograph is findable.
    fn end_run(&mut self) {
        match self.run.as_slice() {
            [] => {}
            [one] => self.out.push(one.to_string()),
            run => self
                .out
                .extend(run.windows(2).map(|pair| pair.iter().collect())),
        }
        self.run.clear();
    }
}

/// Whether `ch` belongs to a script written without spaces between words.
///
/// CJK ideographs and the Japanese kana. **Not Hangul**: Korean is written with spaces, so it
/// tokenizes correctly already and bigramming it would only lose precision.
fn is_scriptio_continua(ch: char) -> bool {
    matches!(ch,
        '\u{3040}'..='\u{30ff}'   // Hiragana and Katakana
        | '\u{3400}'..='\u{4dbf}' // CJK Unified Ideographs Extension A
        | '\u{4e00}'..='\u{9fff}' // CJK Unified Ideographs
        | '\u{f900}'..='\u{faff}' // CJK Compatibility Ideographs
    )
}
