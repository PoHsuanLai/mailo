//! Mark the matched terms, and cut a snippet around the first one.
//!
//! One [`aho_corasick`] automaton, ASCII-case-insensitive, over the terms. Terms that folding
//! changes — `resume` against `Résumé` — are matched on [`search_tokens`]-folded text and mapped
//! back onto the original bytes. Every range is on a char boundary. Overlapping matches merge.

use std::ops::Range;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use mail_domain::filter::search_tokens;

/// Characters of context a snippet keeps before the match.
const BEFORE: usize = 30;
/// Characters of context a snippet keeps after the match.
const AFTER: usize = 60;

/// Byte ranges of `terms` in `text`, char-boundary-safe, overlaps merged.
pub fn marks(text: &str, terms: &[String]) -> Vec<Range<usize>> {
    let patterns: Vec<&str> = terms
        .iter()
        .map(String::as_str)
        .filter(|t| !t.is_empty())
        .collect();
    if patterns.is_empty() || text.is_empty() {
        return Vec::new();
    }

    let mut found = Vec::new();
    if let Some(automaton) = automaton(&patterns) {
        for matched in automaton.find_overlapping_iter(text) {
            found.push(matched.start()..matched.end());
        }
    }

    let folded_terms: Vec<String> = terms
        .iter()
        .flat_map(|term| search_tokens(term))
        .filter(|term| !term.is_empty())
        .collect();
    let folded_refs: Vec<&str> = folded_terms.iter().map(String::as_str).collect();
    let (folded, origin) = fold_map(text);
    if !folded.is_empty()
        && let Some(automaton) = automaton(&folded_refs)
    {
        for matched in automaton.find_overlapping_iter(&folded) {
            if let Some(range) = map_back(text, &folded, &origin, matched.start(), matched.end()) {
                found.push(range);
            }
        }
    }
    merge(found)
}

/// Thirty characters before `first_match` and sixty after, cut on char boundaries.
///
/// A cut end is marked with `…`. A match that already sits at the start or the end of `text`
/// grows no ellipsis on that side.
pub fn snippet(text: &str, first_match: Range<usize>) -> String {
    let range = snap(
        text,
        first_match.start.min(text.len()),
        first_match.end.min(text.len()),
    );
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let start_char = chars
        .iter()
        .position(|(byte, _)| *byte >= range.start)
        .unwrap_or(chars.len());
    let end_char = chars
        .iter()
        .position(|(byte, _)| *byte >= range.end)
        .unwrap_or(chars.len());
    let from = start_char.saturating_sub(BEFORE);
    let to = end_char.saturating_add(AFTER).min(chars.len());

    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    for (_, ch) in chars.iter().take(to).skip(from) {
        out.push(*ch);
    }
    if to < chars.len() {
        out.push('…');
    }
    out
}

fn automaton(patterns: &[&str]) -> Option<AhoCorasick> {
    if patterns.is_empty() {
        return None;
    }
    AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(MatchKind::Standard)
        .build(patterns)
        .ok()
}

/// Folded text, and for each of its chars the byte where that char began in the original.
fn fold_map(text: &str) -> (String, Vec<usize>) {
    let mut folded = String::new();
    let mut origin = Vec::new();
    for (byte, ch) in text.char_indices() {
        let token = search_tokens(&ch.to_string());
        for part in &token {
            for folded_ch in part.chars() {
                origin.push(byte);
                folded.push(folded_ch);
            }
        }
    }
    (folded, origin)
}

fn map_back(
    text: &str,
    folded: &str,
    origin: &[usize],
    start: usize,
    end: usize,
) -> Option<Range<usize>> {
    if start >= end
        || end > folded.len()
        || !folded.is_char_boundary(start)
        || !folded.is_char_boundary(end)
    {
        return None;
    }
    let start_char = folded[..start].chars().count();
    let end_char = folded[..end].chars().count();
    if start_char >= origin.len() || end_char == 0 {
        return None;
    }
    let last = end_char.min(origin.len()) - 1;
    let orig_start = origin[start_char];
    // End at the last matched character. A following space was dropped by folding and must not
    // be swallowed; a combining mark was dropped too, and it belongs to the character it sits on.
    let mut orig_end = char_end(text, origin[last]);
    while orig_end < text.len() {
        let Some(ch) = text[orig_end..].chars().next() else {
            break;
        };
        if !mark_inside_word(ch) {
            break;
        }
        orig_end += ch.len_utf8();
    }
    let range = snap(text, orig_start, orig_end);
    (range.start < range.end).then_some(range)
}

fn char_end(text: &str, start: usize) -> usize {
    match text[start..].chars().next() {
        Some(ch) => start + ch.len_utf8(),
        None => text.len(),
    }
}

/// A combining mark contributes nothing and stays inside the word. A space ends the word.
fn mark_inside_word(ch: char) -> bool {
    let tokens = search_tokens(&format!("q{ch}q"));
    tokens.len() == 1 && tokens[0] == "qq"
}

fn snap(text: &str, mut start: usize, mut end: usize) -> Range<usize> {
    if start > text.len() {
        start = text.len();
    }
    if end > text.len() {
        end = text.len();
    }
    while start < end && !text.is_char_boundary(start) {
        start += 1;
    }
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    start..end
}

fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut out: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = out.last_mut()
            && range.start <= last.end
        {
            if range.end > last.end {
                last.end = range.end;
            }
        } else {
            out.push(range);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{marks, snippet};

    fn terms(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn highlight_table() {
        struct Case {
            name: &'static str,
            text: &'static str,
            terms: &'static [&'static str],
            expect: &'static [&'static str],
        }
        let cases = [
            Case {
                name: "ascii",
                text: "see VALID here",
                terms: &["valid"],
                expect: &["VALID"],
            },
            Case {
                name: "overlap merges",
                text: "abcd",
                terms: &["abc", "bcd"],
                expect: &["abcd"],
            },
            Case {
                name: "cjk",
                text: "校園郵件通知",
                terms: &["郵件"],
                expect: &["郵件"],
            },
            Case {
                name: "emoji before the match",
                text: "😀valid",
                terms: &["valid"],
                expect: &["valid"],
            },
            Case {
                name: "diacritic folded back",
                text: "Résumé",
                terms: &["resume"],
                expect: &["Résumé"],
            },
        ];
        for case in cases {
            let found = marks(case.text, &terms(case.terms));
            assert_eq!(found.len(), case.expect.len(), "{}", case.name);
            for (range, expect) in found.iter().zip(case.expect) {
                assert!(case.text.is_char_boundary(range.start), "{}", case.name);
                assert!(case.text.is_char_boundary(range.end), "{}", case.name);
                assert_eq!(&case.text[range.start..range.end], *expect, "{}", case.name);
            }
        }
        let emoji = marks("😀valid", &terms(&["valid"]));
        assert_eq!(
            emoji[0].start,
            "😀".len(),
            "the match starts after the emoji"
        );
        let preview = snippet("😀valid extra", emoji[0].clone());
        assert!(preview.starts_with('😀'), "{preview}");
        assert!(preview.contains("valid"), "{preview}");
    }

    #[test]
    fn snippets_at_the_ends() {
        let long_after = format!("MATCH{}", "x".repeat(80));
        let at_start = snippet(&long_after, 0..5);
        assert!(at_start.starts_with("MATCH"), "{at_start}");
        assert!(!at_start.starts_with('…'), "{at_start}");
        assert!(at_start.ends_with('…'), "{at_start}");
        assert_eq!(at_start.chars().filter(|ch| *ch == 'x').count(), 60);

        let long_before = format!("{}MATCH", "y".repeat(80));
        let match_at = long_before.len() - "MATCH".len();
        let at_end = snippet(&long_before, match_at..long_before.len());
        assert!(at_end.starts_with('…'), "{at_end}");
        assert!(at_end.ends_with("MATCH"), "{at_end}");
        assert_eq!(at_end.chars().filter(|ch| *ch == 'y').count(), 30);
    }

    #[test]
    fn snippet_keeps_a_preceding_emoji_whole() {
        // Ten 4-byte emoji, then the match. A 30-byte retreat from the match lands inside an
        // emoji; cutting on chars does not.
        let text = format!("{}MATCH{}", "😀".repeat(10), "b".repeat(80));
        let start = "😀".repeat(10).len();
        let preview = snippet(&text, start..start + "MATCH".len());
        assert!(preview.starts_with('😀'), "{preview}");
        assert!(preview.contains("MATCH"), "{preview}");
        assert!(preview.is_char_boundary(0));
    }
}
