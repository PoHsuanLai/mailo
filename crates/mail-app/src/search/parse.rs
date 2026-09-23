//! A search line split into operators, finished words, and the word still being typed.
//!
//! Operators come from [`crate::query::parse_with`], so `from:dana` means the same thing here as
//! it does in the rest of the app. The free words stay separate because prefix expansion
//! completes only the last one, and only while it is still being typed.

use chrono::TimeZone;
use mail_domain::{Filter, LabelId, TextMatch};

/// What one search line is, before it becomes a [`Filter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// Operator filters, in the order they were typed.
    pub operators: Vec<Filter>,
    /// Those operators as typed (`from:dana`), for the menu.
    pub operator_tokens: Vec<String>,
    /// Free words whose trailing whitespace has already been typed.
    pub words: Vec<String>,
    /// Quoted phrases, without the quote marks.
    pub phrases: Vec<String>,
    /// The last free word, when the input does not end in whitespace.
    pub typing: Option<String>,
}

impl Parsed {
    /// The free words, including the one still being typed.
    pub fn query_words(&self) -> Vec<String> {
        let mut words = self.words.clone();
        if let Some(word) = &self.typing {
            words.push(word.clone());
        }
        words
    }

    /// Whether nothing was typed: no operators, no words, no phrase.
    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
            && self.words.is_empty()
            && self.phrases.is_empty()
            && self.typing.is_none()
    }
}

/// Split `input` with [`crate::query::parse_with`].
///
/// `zone` resolves `before:` and `after:`. `label` resolves `label:`. A caller with no index
/// passes `&|_| Vec::new()` and `label:travel` becomes text, which is the parser's own rule.
pub fn parse<Tz: TimeZone>(input: &str, zone: &Tz, label: &dyn Fn(&str) -> Vec<LabelId>) -> Parsed {
    let ended_with_space = input.ends_with(char::is_whitespace);
    let mut operators = Vec::new();
    let mut operator_tokens = Vec::new();
    let mut words = Vec::new();
    let mut phrases = Vec::new();

    for token in raw_tokens(input) {
        match crate::query::parse_with(&token, zone, label) {
            Filter::All => {}
            Filter::Text(TextMatch::Contains(word)) => words.push(word),
            Filter::Text(TextMatch::Exact(phrase)) => phrases.push(phrase),
            other => {
                operators.push(other);
                operator_tokens.push(token);
            }
        }
    }

    // The last free word is still being typed unless whitespace already closed it. A phrase
    // has its closing quote, so it is finished even at the end of the line.
    let typing = if ended_with_space { None } else { words.pop() };

    Parsed {
        operators,
        operator_tokens,
        words,
        phrases,
        typing,
    }
}

/// Whitespace-separated tokens, keeping a quoted run — including `subject:"lunch on friday"` —
/// as one token. The text includes the quotes and a leading `-`, because [`parse`] hands each
/// token to `parse_with` whole.
fn raw_tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut text = String::new();
        text.push(c);
        if c == '"' {
            for ch in chars.by_ref() {
                text.push(ch);
                if ch == '"' {
                    break;
                }
            }
            out.push(text);
            continue;
        }
        while let Some(&next) = chars.peek() {
            if next.is_whitespace() {
                break;
            }
            chars.next();
            text.push(next);
            if next == '"' && text.ends_with(":\"") {
                for ch in chars.by_ref() {
                    text.push(ch);
                    if ch == '"' {
                        break;
                    }
                }
                break;
            }
        }
        if !text.is_empty() {
            out.push(text);
        }
    }
    out
}
