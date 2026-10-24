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
//! - mailo's own names: an identifier, an `#id` or a class that starts `mailo-` is written with
//!   an `x-` before it, and an attribute selector that names `mailo` anywhere is written as one
//!   no element has (`[x-mailo]`), so no rule of the sender's reaches an element of the reader's
//!   own (the frame's inset, a folded quote) by its name; the sanitizer keeps the sender's
//!   markup from carrying those names too (`sanitize.rs`).
//!
//! Strings and identifiers are written back with `cssparser`'s serializer, so `content: "\2022"`
//! survives as the bullet it names.
//!
//! Background images a sender laid out in CSS are therefore never shown, even after the reader
//! allows remote images: only an `<img>`'s address is on the consented list.

use cssparser::{
    ParseError, Parser, ToCss, Token, TokenSerializationType, serialize_identifier,
    serialize_string,
};
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
        && !contains_ci(css, OWN)
}

/// What mailo's own names in a frame's document start with (`mailo-inset`, `mailo-quote`).
const OWN: &str = "mailo";

/// Whether `name` is one of mailo's own: it starts `mailo-`, in any case.
fn is_own(name: &str) -> bool {
    name.get(..OWN.len() + 1)
        .is_some_and(|head| head.eq_ignore_ascii_case("mailo-"))
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
    let mut out = Out {
        css: String::with_capacity(css.len()),
        last: TokenSerializationType::Nothing,
    };
    walk(&mut parser, context, &mut out);
    Cow::Owned(out.css)
}

/// The scrubbed CSS, and the kind of the token written last.
///
/// The walk drops tokens (comments, `<`, `<!--`) and writes some anew (`none`, a number in its
/// shortest form), so two tokens can end up side by side that the parser would read back as one:
/// `a/**/b` would become the identifier `ab`, `a` before `.0` the identifier `a0`. Each write
/// puts the empty comment `/**/` between two such tokens, as CSS's own serializer does, so a
/// second scrub reads the tokens the first one wrote.
struct Out {
    css: String,
    last: TokenSerializationType,
}

impl Out {
    /// Write `text`, the serialization of one token of kind `kind`.
    fn token(&mut self, kind: TokenSerializationType, text: &str) {
        if self.last.needs_separator_when_before(kind) {
            self.css.push_str("/**/");
        }
        self.css.push_str(text);
        self.last = kind;
    }
}

/// Write `parser`'s tokens, read as `context`, into `out`.
fn walk(parser: &mut Parser<'_>, context: Context, out: &mut Out) {
    // A declaration starts at the head of a block and after each `;`.
    let mut declaration_start = true;
    // The at-keyword the rule being read began with, until its block or `;`.
    let mut at_rule: Option<String> = None;
    while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
        match &token {
            Token::WhiteSpace(space) => {
                out.token(token.serialization_type(), space);
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
                Token::SquareBracketBlock if names_own(parser) => {
                    // An attribute selector reaching for mailo's names matches nothing instead.
                    out.token(token.serialization_type(), "[");
                    out.token(TokenSerializationType::Nothing, "x-mailo");
                    out.token(Token::CloseSquareBracket.serialization_type(), "]");
                    continue;
                }
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

/// Whether the bracket block just opened names `mailo` anywhere inside it. The block is read
/// through and the parser put back where it was, so the block is still the caller's to read.
fn names_own(parser: &mut Parser<'_>) -> bool {
    let state = parser.state();
    let start = parser.position();
    // Only read through: what is inside is judged by its text below, and a block that does not
    // parse is still read to its end.
    let _ = parser.parse_nested_block(|inner| {
        while inner.next_including_whitespace_and_comments().is_ok() {}
        Ok::<(), ParseError<()>>(())
    });
    let named = contains_ci(parser.slice_from(start), OWN);
    parser.reset(&state);
    named
}

/// Write one token that is not structure the walk itself reads.
fn write(parser: &mut Parser<'_>, token: &Token<'_>, out: &mut Out) {
    match token {
        Token::Ident(name) | Token::IDHash(name) | Token::Hash(name) if is_own(name) => {
            let mut ident = String::with_capacity(name.len() + 3);
            if !matches!(token, Token::Ident(_)) {
                ident.push('#');
            }
            ident.push_str("x-");
            // Writing to a `String` cannot fail.
            let _ = serialize_identifier(name, &mut ident);
            out.token(token.serialization_type(), &unmarked(&ident));
        }
        Token::Function(name) if FETCHING.iter().any(|f| name.eq_ignore_ascii_case(f)) => {
            out.token(TokenSerializationType::Ident, "none");
            let _ = parser.parse_nested_block(|_| Ok::<(), ParseError<()>>(()));
        }
        Token::UnquotedUrl(_) | Token::BadUrl(_) => {
            out.token(TokenSerializationType::Ident, "none")
        }
        Token::QuotedString(text) => {
            let mut quoted = String::with_capacity(text.len() + 2);
            let _ = serialize_string(text, &mut quoted);
            out.token(token.serialization_type(), &quoted.replace('<', "\\3c "));
        }
        Token::Function(name) => {
            let mut ident = String::with_capacity(name.len());
            let _ = serialize_identifier(name, &mut ident);
            out.token(token.serialization_type(), &unmarked(&ident));
            block(parser, '(', ')', Context::Value, out);
        }
        Token::ParenthesisBlock => block(parser, '(', ')', Context::Value, out),
        Token::SquareBracketBlock => block(parser, '[', ']', Context::Value, out),
        Token::CurlyBracketBlock => block(parser, '{', '}', Context::Value, out),
        other => out.token(
            other.serialization_type(),
            &unmarked(&other.to_css_string()),
        ),
    }
}

/// `css` with each `<` written as the escape `\3c `. An identifier spelling `<` is serialized
/// `\<`, and in a sheet's raw text that `<` still opens a tag: `\3c/style>` would come back out
/// as `\</style>`, ending the sheet and leaving what follows it as markup.
fn unmarked(css: &str) -> Cow<'_, str> {
    if !css.contains('<') {
        return Cow::Borrowed(css);
    }
    Cow::Owned(css.replace("\\<", "\\3c ").replace('<', "\\3c "))
}

/// The block just opened, between `open` and `close`, its inside read as `context`.
fn block(parser: &mut Parser<'_>, open: char, close: char, context: Context, out: &mut Out) {
    let kind = match open {
        '(' => TokenSerializationType::OpenParen,
        _ => TokenSerializationType::Other,
    };
    out.token(kind, open.encode_utf8(&mut [0; 4]));
    let _ = parser.parse_nested_block(|inner| {
        walk(inner, context, out);
        Ok::<(), ParseError<()>>(())
    });
    out.token(
        TokenSerializationType::Other,
        close.encode_utf8(&mut [0; 4]),
    );
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
