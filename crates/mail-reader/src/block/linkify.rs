//! Links in plain text: what a person reads as a link, made one.
//!
//! An address starting `http://`, `https://` or `www.`, and an email address. The address runs
//! to the first space, `<`, `>`, `"` or character outside ASCII, so a link written into Chinese
//! or Japanese text ends where the text resumes. Punctuation that ends a sentence comes off
//! its end, as does a closing bracket the address did not open: `(see https://a.test/x)` links
//! `https://a.test/x`, and `https://a.test/wiki/A_(b)` keeps its `)`.
//!
//! Every link goes through [`SafeUrl::link`], so a plain-text link is held to the same three
//! schemes, and loses the same tracking parameters, as one in HTML. Text that does not parse
//! stays text.

use super::kind::Span;
use super::url::SafeUrl;

/// `text` as spans: text, and links where it has them.
pub(super) fn linked(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain = 0;
    let mut at = 0;
    while at < text.len() {
        let Some((start, end, href)) = next_link(text, at) else {
            break;
        };
        if let Some(url) = SafeUrl::link(&href) {
            if plain < start {
                spans.push(Span::Text(text[plain..start].to_owned()));
            }
            spans.push(Span::Link {
                url,
                spans: vec![Span::Text(text[start..end].to_owned())],
            });
            plain = end;
        }
        at = end;
    }
    if plain < text.len() || spans.is_empty() {
        spans.push(Span::Text(text[plain..].to_owned()));
    }
    spans
}

/// The next link at or after `from`: where it starts and ends in `text`, and the URL it opens.
fn next_link(text: &str, from: usize) -> Option<(usize, usize, String)> {
    let mut index = from;
    while index < text.len() {
        if let Some(found) = web_at(text, index).or_else(|| email_at(text, index)) {
            return Some(found);
        }
        index += text[index..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// A web address starting at `start`.
fn web_at(text: &str, start: usize) -> Option<(usize, usize, String)> {
    if !starts_word(text, start) {
        return None;
    }
    let rest = &text[start..];
    let prefix = ["https://", "http://", "www."].into_iter().find(|prefix| {
        rest.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    })?;
    let end = start + trimmed(&rest[..run(rest)]);
    if end <= start + prefix.len() {
        return None;
    }
    let shown = &text[start..end];
    let href = if prefix == "www." {
        format!("https://{shown}")
    } else {
        shown.to_owned()
    };
    Some((start, end, href))
}

/// An email address whose local part starts at `start`.
fn email_at(text: &str, start: usize) -> Option<(usize, usize, String)> {
    if !starts_word(text, start) {
        return None;
    }
    let rest = &text[start..];
    let local = rest.bytes().take_while(|byte| is_local(*byte)).count();
    if local == 0 || rest.as_bytes().get(local) != Some(&b'@') {
        return None;
    }
    let domain = &rest[local + 1..];
    let length = domain
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        .count();
    let domain = domain[..length].trim_end_matches(['.', '-']);
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| label.is_empty()) {
        return None;
    }
    let end = start + local + 1 + domain.len();
    Some((start, end, format!("mailto:{}", &text[start..end])))
}

/// Nothing that could belong to the same address comes just before `at`. A letter outside
/// ASCII may: no address has one, and Chinese or Japanese runs straight into a link.
fn starts_word(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(|ch| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '@' | '/' | '+' | '%'))
    })
}

fn is_local(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-')
}

/// How far an address runs: to a space, a character outside ASCII, or one that ends it.
fn run(rest: &str) -> usize {
    rest.char_indices()
        .find(|(_, ch)| !ch.is_ascii_graphic() || matches!(ch, '<' | '>' | '"' | '`'))
        .map_or(rest.len(), |(index, _)| index)
}

/// `address` without what ends the sentence around it rather than the address.
///
/// Each bracket kind is counted once, and a close is taken off its count as it is trimmed, so a
/// run of thousands of closing brackets costs one pass rather than one pass per bracket.
fn trimmed(address: &str) -> usize {
    const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
    let mut opened = [0usize; 3];
    let mut closed = [0usize; 3];
    for ch in address.chars() {
        for (kind, (open, close)) in PAIRS.iter().enumerate() {
            if ch == *open {
                opened[kind] += 1;
            } else if ch == *close {
                closed[kind] += 1;
            }
        }
    }
    let mut end = address.len();
    loop {
        let Some(last) = address[..end].chars().next_back() else {
            return end;
        };
        let drop = match PAIRS.iter().position(|(_, close)| *close == last) {
            Some(kind) => {
                let unopened = opened[kind] < closed[kind];
                if unopened {
                    closed[kind] -= 1;
                }
                unopened
            }
            None => matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*'),
        };
        if !drop {
            return end;
        }
        end -= last.len_utf8();
    }
}
