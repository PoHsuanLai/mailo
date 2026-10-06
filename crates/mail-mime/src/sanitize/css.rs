//! The sender's CSS, kept for the reader's frame with everything that could reach the network
//! or run taken out.
//!
//! The frame's own network refuses what the reader did not consent to, so this is the second
//! wall, not the first: a renderer that fetched whatever CSS named would still be handed nothing
//! to fetch. The CSS is tokenized with `cssparser` (the tokenizer the renderer's own engine
//! uses, so an escape such as `u\72l(` is decoded here exactly as it would be there) and written
//! back token by token, leaving out:
//! - every function that takes a URL or a string that is one (`url()`, `src()`, `image()`,
//!   `image-set()`, `cross-fade()`, `element()`) and an unquoted `url(...)` token, each written
//!   as `none`, and `expression()`, which an old engine ran as code;
//! - `@import` rules, which load another sheet;
//! - declarations of `behavior` and `-moz-binding`, matched as a declaration's property name,
//!   never inside a selector or a longer name;
//! - comments, bad tokens, `<!--`/`-->` and a bare `<`: CSS has no use for them, and a `<` is
//!   the only way a sheet's text could read as markup. A `<` inside a string is written escaped.
//!
//! Strings and identifiers are written back with `cssparser`'s serializer, so `content: "\2022"`
//! survives as the bullet it names.
//!
//! Background images a sender laid out in CSS are therefore never shown, even after the reader
//! allows remote images: only an `<img>`'s address is on the consented list.

use cssparser::{ParseError, Parser, ToCss, Token, serialize_identifier, serialize_string};
use std::borrow::Cow;

/// Functions whose arguments name something to fetch or run. Each call becomes `none`.
const FETCHING: &[&str] = &[
    "url",
    "src",
    "image",
    "image-set",
    "-webkit-image-set",
    "cross-fade",
    "-webkit-cross-fade",
    "element",
    "-moz-element",
    "expression",
];

/// Properties that are code in some engine: the whole declaration goes.
const RUNNING: &[&str] = &["behavior", "-moz-binding"];

/// At-rules whose block holds rules rather than declarations.
const GROUPING: &[&str] = &[
    "media",
    "supports",
    "document",
    "-moz-document",
    "layer",
    "container",
    "scope",
    "starting-style",
    "keyframes",
    "-webkit-keyframes",
];

/// What a run of CSS is: a sheet's rules, a block's declarations (a `style` attribute is one),
/// or the inside of a function or a bracket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Context {
    Rules,
    Declarations,
    Value,
}

/// Whether `css` has nothing the walk would change: no escape, no `<`, no at-rule, no comment, no
/// function or url token, and neither code-running property. Most inline styles are like this.
fn plainly_safe(css: &str) -> bool {
    !css.bytes()
        .any(|b| matches!(b, b'\\' | b'<' | b'@' | b'(' | b'/'))
        && !RUNNING.iter().any(|name| contains_ci(css, name))
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// `css`, read as `context`, with nothing in it that fetches or runs.
pub(super) fn scrub(css: &str, context: Context) -> Cow<'_, str> {
    if plainly_safe(css) {
        return Cow::Borrowed(css);
    }
    let mut parser = Parser::new(css);
    let mut out = String::with_capacity(css.len());
    walk(&mut parser, context, &mut out);
    Cow::Owned(out)
}

/// Write `parser`'s tokens, read as `context`, into `out`.
fn walk(parser: &mut Parser<'_>, context: Context, out: &mut String) {
    // A declaration starts at the head of a block and after each `;`.
    let mut declaration_start = true;
    // The at-keyword the rule being read began with, until its block or `;`.
    let mut at_rule: Option<String> = None;
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        match &token {
            Token::WhiteSpace(space) => {
                out.push_str(space);
                continue;
            }
            Token::Comment(_)
            | Token::CDO
            | Token::CDC
            | Token::BadString(_)
            | Token::Delim('<') => {
                continue;
            }
            _ => {}
        }
        match context {
            Context::Rules => match &token {
                Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => {
                    skip_declaration(parser);
                    continue;
                }
                Token::AtKeyword(name) => at_rule = Some(name.to_ascii_lowercase()),
                Token::Semicolon => at_rule = None,
                Token::CurlyBracketBlock => {
                    let inner = match at_rule.take() {
                        Some(name) if GROUPING.contains(&name.as_str()) => Context::Rules,
                        _ => Context::Declarations,
                    };
                    block(parser, '{', '}', inner, out);
                    continue;
                }
                _ => {}
            },
            Context::Declarations => {
                if declaration_start
                    && let Token::Ident(name) = &token
                    && RUNNING
                        .iter()
                        .any(|running| name.eq_ignore_ascii_case(running))
                {
                    skip_declaration(parser);
                    continue;
                }
                declaration_start = matches!(token, Token::Semicolon);
                if matches!(token, Token::CurlyBracketBlock) {
                    // A nested rule's block (`&:hover { ... }`).
                    block(parser, '{', '}', Context::Declarations, out);
                    declaration_start = true;
                    continue;
                }
            }
            Context::Value => {}
        }
        write(parser, &token, out);
    }
}

/// Write one token that is not structure the walk itself reads.
fn write(parser: &mut Parser<'_>, token: &Token<'_>, out: &mut String) {
    match token {
        Token::Function(name) if FETCHING.iter().any(|f| name.eq_ignore_ascii_case(f)) => {
            out.push_str("none");
            let _ = parser.parse_nested_block(|_| Ok::<(), ParseError<()>>(()));
        }
        Token::UnquotedUrl(_) | Token::BadUrl(_) => out.push_str("none"),
        Token::QuotedString(text) => {
            let mut quoted = String::with_capacity(text.len() + 2);
            let _ = serialize_string(text, &mut quoted);
            out.push_str(&quoted.replace('<', "\\3c "));
        }
        Token::Function(name) => {
            let _ = serialize_identifier(name, out);
            block(parser, '(', ')', Context::Value, out);
        }
        Token::ParenthesisBlock => block(parser, '(', ')', Context::Value, out),
        Token::SquareBracketBlock => block(parser, '[', ']', Context::Value, out),
        Token::CurlyBracketBlock => block(parser, '{', '}', Context::Value, out),
        other => out.push_str(&other.to_css_string()),
    }
}

/// The block just opened, between `open` and `close`, its inside read as `context`.
fn block(parser: &mut Parser<'_>, open: char, close: char, context: Context, out: &mut String) {
    out.push(open);
    let _ = parser.parse_nested_block(|inner| {
        walk(inner, context, out);
        Ok::<(), ParseError<()>>(())
    });
    out.push(close);
}

/// Pass over the rest of a declaration or an at-rule's prelude, through its `;` or to the end of
/// the block it is in (the parser steps over nested blocks whole).
fn skip_declaration(parser: &mut Parser<'_>) {
    while let Ok(token) = parser.next() {
        if matches!(token, Token::Semicolon) {
            break;
        }
    }
}

/// Every `<style>` element's text in markup the sanitizer serialized, scrubbed as a sheet. The
/// serializer writes a style element's text raw and escapes `<` in attribute values, and the
/// parser ended the element at the first `</style`, so the text between an opening tag and the
/// next `</style>` is the whole sheet.
pub(super) fn scrub_style_elements(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = find_ci(rest, "<style") {
        let Some(tag_end) = rest[open..].find('>').map(|end| open + end + 1) else {
            break;
        };
        out.push_str(&rest[..tag_end]);
        let body = &rest[tag_end..];
        let close = find_ci(body, "</style").unwrap_or(body.len());
        out.push_str(&scrub(&body[..close], Context::Rules));
        rest = &body[close..];
    }
    out.push_str(rest);
    out
}

/// Where `needle` (ASCII) first occurs in `hay`, ignoring ASCII case.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let n = needle.len();
    hay.char_indices().map(|(i, _)| i).find(|&i| {
        hay.get(i..i + n)
            .is_some_and(|s| s.eq_ignore_ascii_case(needle))
    })
}

#[cfg(test)]
#[path = "css_tests.rs"]
mod tests;
