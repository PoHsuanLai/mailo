//! Fuzzy match over the small sets: candidate subjects, people, labels, places, commands.
//!
//! The body index stays exact. Typos are forgiven here, where the list is already small.
//!
//! **Matcher: `nucleo-matcher` 0.3 (MPL-2.0).** Measured against `frizbee` 0.13 (MIT) on 10 000
//! synthetic subjects (`"Quarterly report {n} …"`) and the 6-character typo `reprot` of
//! `report`, release build, five runs after a warmup:
//!
//! | matcher | what was timed | median of 5, after warmup |
//! |---|---|---|
//! | nucleo-matcher | `Pattern::indices` | 8.1 ms (5.3–13.7) |
//! | frizbee | `match_list_indices` | 2.7 ms (2.7–5.6) |
//!
//! Frizbee is faster. It is not the one this module calls, because the menu has to highlight
//! the matched characters and frizbee's indices are not those characters once the text leaves
//! ASCII. On `郵件` against `校園郵件通知` it reported byte offsets `6..12`. On `resume`
//! against `Résumé` it skipped the `é` and the `s`. `nucleo-matcher` reported char indices for
//! both: `[2, 3]` and `[0, 1, 2, 3, 4, 5]`, and `Normalization::Smart` is what makes `resume`
//! find `Résumé` at all. Six milliseconds for ten thousand subjects is inside a keystroke; the
//! menu matches hundreds, not thousands. MPL-2.0 is a link dependency (`deny.toml` allows it)
//! and none of its source is copied here.
//!
//! The ignored `fuzzy_bench` test remeasures this crate's `match_list` (indices included) on
//! the same 10 000 subjects.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// One haystack the query matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyHit {
    /// Index into the slice passed to [`match_list`].
    pub index: usize,
    /// Higher is a closer match. Nucleo's own score.
    pub score: u32,
    /// Char indices into the haystack, low to high.
    pub indices: Vec<u32>,
}

/// Fuzzy-match `query` against `haystacks`, best score first.
///
/// An empty query matches nothing: the empty menu is the group's job, not a match against
/// every row. A query that shares no sequence with a haystack is absent from the result.
pub fn match_list(query: &str, haystacks: &[&str]) -> Vec<FuzzyHit> {
    let query = query.trim();
    if query.is_empty() || haystacks.is_empty() {
        return Vec::new();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut buf = Vec::new();
    let mut hits = Vec::new();
    for (index, hay) in haystacks.iter().enumerate() {
        buf.clear();
        let mut indices = Vec::new();
        let utf = Utf32Str::new(hay, &mut buf);
        if let Some(score) = pattern.indices(utf, &mut matcher, &mut indices) {
            indices.sort_unstable();
            indices.dedup();
            hits.push(FuzzyHit {
                index,
                score,
                indices,
            });
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    hits
}

#[cfg(test)]
mod tests {
    use super::match_list;

    #[test]
    fn fuzzy_table() {
        struct Case {
            name: &'static str,
            query: &'static str,
            haystacks: &'static [&'static str],
            first: Option<usize>,
        }
        let cases = [
            Case {
                name: "typo",
                query: "uidvaldity",
                haystacks: &["noise", "UIDVALIDITY"],
                first: Some(1),
            },
            Case {
                name: "case",
                query: "uidvalidity",
                haystacks: &["UIDVALIDITY"],
                first: Some(0),
            },
            Case {
                name: "diacritic",
                query: "resume",
                haystacks: &["notes", "Résumé"],
                first: Some(1),
            },
            Case {
                name: "cjk",
                query: "郵件",
                haystacks: &["hello", "校園郵件通知"],
                first: Some(1),
            },
            Case {
                name: "no match",
                query: "zzzz",
                haystacks: &["hello", "UIDVALIDITY", "校園郵件通知"],
                first: None,
            },
        ];
        for case in cases {
            let hits = match_list(case.query, case.haystacks);
            match case.first {
                None => assert!(hits.is_empty(), "{}: {hits:?}", case.name),
                Some(index) => {
                    let hit = hits
                        .first()
                        .unwrap_or_else(|| panic!("{}: no hit", case.name));
                    assert_eq!(hit.index, index, "{}", case.name);
                    let hay = case.haystacks[index];
                    let chars = hay.chars().count();
                    for offset in &hit.indices {
                        assert!(
                            (*offset as usize) < chars,
                            "{}: index {offset} is not a char of {hay:?}",
                            case.name
                        );
                    }
                    let mut sorted = hit.indices.clone();
                    sorted.sort_unstable();
                    assert_eq!(
                        hit.indices, sorted,
                        "{}: indices are low to high",
                        case.name
                    );
                }
            }
        }
    }

    /// 10 000 synthetic subjects, 6-character typo of `report`. Release numbers are what the
    /// module docs cite; this is the check that the chosen matcher still runs.
    #[test]
    #[ignore]
    fn fuzzy_bench() {
        let subjects: Vec<String> = (0..10_000)
            .map(|i| {
                format!(
                    "Quarterly report {i} for the finance team about invoice {}",
                    i % 97
                )
            })
            .collect();
        let refs: Vec<&str> = subjects.iter().map(String::as_str).collect();
        let query = "reprot";
        let started = std::time::Instant::now();
        let hits = match_list(query, &refs);
        let elapsed = started.elapsed();
        eprintln!(
            "nucleo-matcher match_list: {} subjects, query {query:?}: {elapsed:?}, {} hits",
            subjects.len(),
            hits.len()
        );
        assert!(!hits.is_empty());
    }
}
