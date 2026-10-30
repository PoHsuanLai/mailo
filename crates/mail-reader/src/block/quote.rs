//! Attribution lines.
//!
//! Recognised only by the caller, and only when the line is immediately
//! followed by a quote. The match itself is a phrase at the end of the line,
//! in six languages mail actually uses, on a word boundary. `rewrote` does
//! not match `wrote`. A match inside a URL does not count. A line of dashes
//! does not count: it is a rule, and rules are not people.

/// Whether `line` is the "so-and-so wrote:" line in front of a quote.
pub(crate) fn is_attribution(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() || is_dash_rule(line) {
        return false;
    }
    PHRASES.iter().any(|phrase| matches_phrase(line, phrase))
}

/// English, French, German, Spanish, Portuguese, Chinese.
const PHRASES: &[&str] = &[
    "wrote",
    "a écrit",
    "a ecrit",
    "schrieb",
    "escribió",
    "escribio",
    "escreveu",
    "写道",
];

/// The phrase, then at most a short name and a colon.
///
/// "Ada wrote:" and "schrieb Cy:" are both how a client introduces a quote.
/// A sentence that merely contains the verb has more words after it, and is
/// left alone.
fn matches_phrase(line: &str, phrase: &str) -> bool {
    let line = line.to_lowercase();
    let phrase = phrase.to_lowercase();
    let mut from = 0;
    while let Some(rel) = line.get(from..).and_then(|rest| rest.find(&phrase)) {
        let at = from + rel;
        let end = at + phrase.len();
        let before = line[..at].chars().next_back();
        let after_char = line.get(end..).and_then(|rest| rest.chars().next());
        let bounded = before.is_none_or(|ch| !ch.is_alphanumeric())
            && after_char.is_none_or(|ch| !ch.is_alphanumeric());
        if bounded && !match_inside_url(&line, at) {
            let trailing = line[end..].trim().trim_end_matches([':', '：']).trim();
            if trailing.split_whitespace().count() <= 3 {
                return true;
            }
        }
        let next = end.max(from + 1);
        if next <= from {
            break;
        }
        from = next;
    }
    false
}

fn match_inside_url(line: &str, index: usize) -> bool {
    let start = line[..index]
        .rfind(char::is_whitespace)
        .map(|at| at + line[at..].chars().next().map(char::len_utf8).unwrap_or(0))
        .unwrap_or(0);
    let rest = line.get(index..).unwrap_or("");
    let end = rest
        .find(char::is_whitespace)
        .map(|at| index + at)
        .unwrap_or(line.len());
    let Some(token) = line.get(start..end) else {
        return false;
    };
    token.contains("://") || token.starts_with("www.") || token.starts_with("mailto:")
}

fn is_dash_rule(line: &str) -> bool {
    let line = line.trim();
    line.len() >= 3
        && line.chars().any(|ch| matches!(ch, '-' | '_' | '=' | '*'))
        && line
            .chars()
            .all(|ch| matches!(ch, '-' | '_' | '=' | '*' | ' ' | '\t'))
}
