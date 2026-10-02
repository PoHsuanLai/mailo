//! `re:/pattern/` over the candidate set.
//!
//! The `regex` crate is linear-time, and the compiled form is capped, so a pattern cannot hang
//! the window. An invalid pattern is the crate's own error text. It is never a panic.

use regex::RegexBuilder;

/// Compiled size cap, and the same cap on the lazy DFA. One mebibyte.
const SIZE_LIMIT: usize = 1 << 20;

/// `input` with at most one `re:/pattern/` removed, and the compiled pattern when there was one.
#[derive(Debug, Clone)]
pub struct Extracted {
    pub rest: String,
    pub regex: Option<regex::Regex>,
}

/// Pull `re:/pattern/` out of `input`.
///
/// No such clause is `Ok` with `regex: None`. A pattern the `regex` crate refuses — a broken
/// class, a program past [`SIZE_LIMIT`] — is `Err` of that crate's message.
pub fn extract(input: &str) -> Result<Extracted, String> {
    let Some(start) = find_re(input) else {
        return Ok(Extracted {
            rest: input.to_owned(),
            regex: None,
        });
    };
    // `re:/` is four ASCII bytes, and `start` sits on the `r`, so this is a char boundary.
    let after = start + 4;
    let tail = &input[after..];
    let (pattern, consumed) = match tail.find('/') {
        Some(end) => (&tail[..end], end + 1),
        None => (tail, tail.len()),
    };
    let regex = compile(pattern)?;
    let mut rest = String::with_capacity(input.len().saturating_sub(pattern.len()));
    rest.push_str(&input[..start]);
    rest.push_str(&tail[consumed..]);
    Ok(Extracted {
        rest,
        regex: Some(regex),
    })
}

/// Whether `regex` occurs in the subject or the snippet. Bodies outside those two are not scanned.
pub fn hits(regex: &regex::Regex, subject: &str, snippet: &str) -> bool {
    regex.is_match(subject) || regex.is_match(snippet)
}

/// Compile `pattern` with the size caps. The error string belongs to the `regex` crate.
pub fn compile(pattern: &str) -> Result<regex::Regex, String> {
    RegexBuilder::new(pattern)
        .size_limit(SIZE_LIMIT)
        .dfa_size_limit(SIZE_LIMIT)
        .build()
        .map_err(|err| err.to_string())
}

/// Byte index of a `re:/` that starts a token. ASCII, so the index is a char boundary.
fn find_re(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let at_boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();
        let is_re = bytes[i].eq_ignore_ascii_case(&b'r')
            && bytes[i + 1].eq_ignore_ascii_case(&b'e')
            && bytes[i + 2] == b':'
            && bytes[i + 3] == b'/';
        if at_boundary && is_re {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{compile, extract, hits};

    #[test]
    fn pathological_pattern_finishes() {
        let regex = compile("(a+)+$").expect("the pattern is valid");
        let text = format!("{}b", "a".repeat(30));
        let started = Instant::now();
        let matched = hits(&regex, &text, "");
        assert!(started.elapsed() < Duration::from_millis(50));
        assert!(!matched);
    }

    #[test]
    fn unclosed_class_is_the_regex_error() {
        let err = extract("re:/[/").expect_err("the class is unclosed");
        // Built at runtime so the literal is not a regex clippy can reject.
        let broken = String::from("[");
        let own = regex::Regex::new(&broken)
            .expect_err("same pattern")
            .to_string();
        assert_eq!(err, own);
    }

    #[test]
    fn size_limit_breach_is_an_error() {
        // A short pattern whose compiled NFA is large. A literal alternation is compressed; a
        // long counted class is not, and it crosses the 1 MiB cap `compile` sets.
        let err = compile("[a-zA-Z0-9]{100000}").expect_err("the program must exceed the cap");
        assert!(
            err.to_ascii_lowercase().contains("size"),
            "the message should be the size-limit error, got {err}"
        );
    }
}
