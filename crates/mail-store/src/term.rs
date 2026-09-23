//! One term from the full-text vocabulary.

/// A term the index holds, with how often it occurs.
///
/// `cjk_bigram` is true for the overlapping two-character tokens the index stores for CJK
/// ideographs and Japanese kana. Callers expand a query with those and never show one as a
/// suggested word. A single ideograph, which the index stores when a run has length one, is
/// not a bigram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub text: String,
    /// How many messages contain the term. This is FTS5's `doc` count, not the number of times
    /// the term is repeated inside a message.
    pub docs: u64,
    pub cjk_bigram: bool,
}

/// Most frequent first. Equal counts break by the term text, so two stores agree on the order.
pub(crate) fn by_frequency(terms: &mut [Term]) {
    terms.sort_by(|a, b| b.docs.cmp(&a.docs).then_with(|| a.text.cmp(&b.text)));
}
