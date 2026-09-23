//! Prefix expansion that does not change what [`Filter::Text`] means.
//!
//! The last free word, while it is still being typed, is folded with [`search_tokens`] and
//! completed from the index vocabulary. The query becomes `And(earlier words, Or(Text(term)))`,
//! each `Text` still a whole word. A bigram expands the query and is not offered as a suggestion.

use mail_domain::filter::search_tokens;
use mail_domain::{Filter, TextMatch};

use super::parsing::Parsed;
use super::source::{Source, Term};

/// How many vocabulary entries one prefix may fan out to.
const PREFIX_LIMIT: usize = 32;

/// The filter prefix expansion built, the terms to mark, and the words to suggest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    pub filter: Filter,
    /// Whole words and phrases to highlight. Bigrams included: they matched, they are just not
    /// suggestions.
    pub needles: Vec<String>,
    /// Vocabulary hits for the word being typed, most frequent first, bigrams removed.
    pub suggestions: Vec<Term>,
}

/// Build the filter for `parsed`.
pub fn expand(parsed: &Parsed, source: &dyn Source) -> Expansion {
    let mut clauses = parsed.operators.clone();
    let mut needles = Vec::new();

    for phrase in &parsed.phrases {
        clauses.push(Filter::Text(TextMatch::Exact(phrase.clone())));
        needles.push(phrase.clone());
    }
    for word in &parsed.words {
        clauses.push(text(word));
        needles.push(word.clone());
    }

    let suggestions = if let Some(word) = &parsed.typing {
        let (clause, typed_needles, suggestions) = expand_word(word, source);
        clauses.push(clause);
        needles.extend(typed_needles);
        suggestions
    } else {
        Vec::new()
    };

    Expansion {
        filter: and_all(clauses),
        needles,
        suggestions,
    }
}

/// The suggestions [`expand`] would offer. Bigrams are absent.
pub fn suggestions(parsed: &Parsed, source: &dyn Source) -> Vec<Term> {
    expand(parsed, source).suggestions
}

/// Fold `word` and complete its last token. No vocabulary hit means the word itself, unchanged.
fn expand_word(word: &str, source: &dyn Source) -> (Filter, Vec<String>, Vec<Term>) {
    let tokens = search_tokens(word);
    let Some((prefix, earlier)) = tokens.split_last() else {
        return (text(word), vec![word.to_owned()], Vec::new());
    };

    let terms = vocabulary(prefix, source);
    // One letter completes to nothing. Its completions are the commonest words that start with
    // it — `us`, `up`, `use`, `update` — whose union is most of the index, and ranking most of
    // the index is what a keystroke cannot afford. `tests/search_scale.rs` measured `u` at
    // 1.2 s over 50,000 messages whose `u` words were that common, and `us` alone at 62 ms over
    // its Zipf-distributed mailbox. One ideograph is already a word, and the index holds it as
    // bigrams, so a single CJK character still completes to those.
    let terms: Vec<Term> = if prefix.chars().count() == 1 {
        terms.into_iter().filter(|term| term.cjk_bigram).collect()
    } else {
        terms
    };
    let suggestions = terms
        .iter()
        .filter(|term| !term.cjk_bigram)
        .cloned()
        .collect();
    if terms.is_empty() {
        return (text(word), vec![word.to_owned()], suggestions);
    }

    let mut needles: Vec<String> = earlier.to_vec();
    needles.extend(terms.iter().map(|term| term.text.clone()));

    let mut clauses: Vec<Filter> = earlier.iter().map(|token| text(token)).collect();
    let alternatives: Vec<Filter> = terms.iter().map(|term| text(&term.text)).collect();
    clauses.push(Filter::Or(alternatives));
    (and_all(clauses), needles, suggestions)
}

fn vocabulary(prefix: &str, source: &dyn Source) -> Vec<Term> {
    let mut terms = source.terms_with_prefix(prefix, PREFIX_LIMIT);
    // Most frequent first. Equal counts stay in text order so a test can name the sequence.
    terms.sort_by(|a, b| b.docs.cmp(&a.docs).then(a.text.cmp(&b.text)));
    terms.truncate(PREFIX_LIMIT);
    terms
}

fn text(word: &str) -> Filter {
    Filter::Text(TextMatch::Contains(word.to_owned()))
}

fn and_all(mut clauses: Vec<Filter>) -> Filter {
    match clauses.len() {
        0 => Filter::All,
        1 => clauses.remove(0),
        _ => Filter::And(clauses),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use chrono::Utc;
    use mail_domain::{Filter, TextMatch, ThreadSummary};

    use super::super::Parsed;
    use super::super::source::{Source, Term};
    use super::{expand, suggestions};
    use crate::search::parse;

    struct Vocab {
        terms: Vec<Term>,
        calls: Cell<usize>,
    }

    impl Source for Vocab {
        fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Vec<Term> {
            self.calls.set(self.calls.get() + 1);
            self.terms
                .iter()
                .filter(|term| term.text.starts_with(prefix))
                .take(limit)
                .cloned()
                .collect()
        }

        fn ranked(
            &self,
            _: &Filter,
            _: usize,
            _: chrono::DateTime<Utc>,
        ) -> Vec<(ThreadSummary, f64)> {
            Vec::new()
        }
    }

    fn vocab(terms: Vec<Term>) -> Vocab {
        Vocab {
            terms,
            calls: Cell::new(0),
        }
    }

    fn latin() -> Vec<Term> {
        vec![
            Term {
                text: "validity".to_owned(),
                docs: 5,
                cjk_bigram: false,
            },
            Term {
                text: "valid".to_owned(),
                docs: 9,
                cjk_bigram: false,
            },
            Term {
                text: "validation".to_owned(),
                docs: 2,
                cjk_bigram: false,
            },
        ]
    }

    fn bigrams() -> Vec<Term> {
        vec![
            Term {
                text: "電子".to_owned(),
                docs: 4,
                cjk_bigram: true,
            },
            Term {
                text: "電話".to_owned(),
                docs: 8,
                cjk_bigram: true,
            },
        ]
    }

    fn parsed(input: &str) -> Parsed {
        parse(input, &Utc, &|_| Vec::new())
    }

    fn contains(filter: &Filter) -> Vec<String> {
        match filter {
            Filter::Text(TextMatch::Contains(word)) => vec![word.clone()],
            Filter::Or(parts) | Filter::And(parts) => parts.iter().flat_map(contains).collect(),
            Filter::From(TextMatch::Contains(word)) => vec![word.clone()],
            other => panic!("unexpected filter {other:?}"),
        }
    }

    #[test]
    fn expand_table() {
        struct Case {
            name: &'static str,
            input: &'static str,
            terms: fn() -> Vec<Term>,
            /// `Some` when the whole filter is an `Or` of these texts, most frequent first.
            or: Option<&'static [&'static str]>,
            /// `Some` when the filter is exactly `Text(Contains(this))`.
            text: Option<&'static str>,
            earlier: &'static [&'static str],
            suggestions: &'static [&'static str],
            calls: bool,
        }
        let cases = [
            Case {
                name: "three whole-word terms",
                input: "valid",
                terms: latin,
                or: Some(&["valid", "validity", "validation"]),
                text: None,
                earlier: &[],
                suggestions: &["valid", "validity", "validation"],
                calls: true,
            },
            Case {
                name: "folded prefix",
                input: "Valid",
                terms: latin,
                or: Some(&["valid", "validity", "validation"]),
                text: None,
                earlier: &[],
                suggestions: &["valid", "validity", "validation"],
                calls: true,
            },
            Case {
                name: "no vocabulary",
                input: "valid",
                terms: Vec::new,
                or: None,
                text: Some("valid"),
                earlier: &[],
                suggestions: &[],
                calls: true,
            },
            Case {
                name: "earlier word kept",
                input: "invoice valid",
                terms: latin,
                or: Some(&["valid", "validity", "validation"]),
                text: None,
                earlier: &["invoice"],
                suggestions: &["valid", "validity", "validation"],
                calls: true,
            },
            Case {
                name: "cjk bigrams expand but are not suggested",
                input: "電",
                terms: bigrams,
                or: Some(&["電話", "電子"]),
                text: None,
                earlier: &[],
                suggestions: &[],
                calls: true,
            },
            Case {
                name: "one latin letter does not complete",
                input: "V",
                terms: latin,
                or: None,
                text: Some("V"),
                earlier: &[],
                suggestions: &[],
                calls: true,
            },
            Case {
                name: "trailing space does not expand",
                input: "valid ",
                terms: latin,
                or: None,
                text: Some("valid"),
                earlier: &[],
                suggestions: &[],
                calls: false,
            },
        ];

        for case in cases {
            let source = vocab((case.terms)());
            let parsed = parsed(case.input);
            let expansion = expand(&parsed, &source);
            let offered = suggestions(&parsed, &source);
            if !case.calls {
                assert_eq!(source.calls.get(), 0, "{}", case.name);
            }
            if let Some(text) = case.text {
                assert!(
                    matches!(&expansion.filter, Filter::Text(TextMatch::Contains(word)) if word == text),
                    "{}: {:?}",
                    case.name,
                    expansion.filter
                );
            }
            if let Some(or) = case.or {
                let (Filter::Or(parts) | Filter::And(parts)) = &expansion.filter else {
                    panic!(
                        "{}: expected Or or And, got {:?}",
                        case.name, expansion.filter
                    );
                };
                let texts = contains(&Filter::Or(parts.clone()));
                for word in or {
                    assert!(
                        texts.iter().any(|got| got == word),
                        "{name}: missing {word} in {texts:?}",
                        name = case.name
                    );
                }
                // The Or itself is most-frequent-first, not buried under an earlier word.
                let or_filter = parts.iter().find_map(|part| match part {
                    Filter::Or(alts) => Some(alts),
                    Filter::Text(_) if parts.len() == 1 => None,
                    _ => None,
                });
                let alts = match (&expansion.filter, or_filter) {
                    (Filter::Or(alts), _) => alts.clone(),
                    (_, Some(alts)) => alts.clone(),
                    _ => panic!("{}: no Or in {:?}", case.name, expansion.filter),
                };
                let got: Vec<&str> = alts
                    .iter()
                    .map(|alt| match alt {
                        Filter::Text(TextMatch::Contains(word)) => word.as_str(),
                        other => panic!("{}: {other:?}", case.name),
                    })
                    .collect();
                assert_eq!(got, or, "{}", case.name);
            }
            let texts = contains(&expansion.filter);
            for word in case.earlier {
                assert!(
                    texts.iter().any(|got| got == word),
                    "{}: dropped {word}",
                    case.name
                );
            }
            let offered_text: Vec<&str> = offered.iter().map(|term| term.text.as_str()).collect();
            assert_eq!(offered_text, case.suggestions, "{}", case.name);
            assert!(offered.iter().all(|term| !term.cjk_bigram), "{}", case.name);
        }
    }

    #[test]
    fn operators_stay_operators() {
        let source = vocab(latin());
        let expansion = expand(&parsed("from:ada valid"), &source);
        let Filter::And(parts) = &expansion.filter else {
            panic!("{:?}", expansion.filter);
        };
        assert!(matches!(parts[0], Filter::From(_)), "{parts:?}");
        assert!(matches!(parts[1], Filter::Or(_)), "{parts:?}");
    }
}
