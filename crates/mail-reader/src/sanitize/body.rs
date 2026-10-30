//! The sender's `<body>` styling, which the sanitizer's fragment parse drops with the element:
//! a newsletter's full-bleed ground is usually `<body bgcolor=…>` or `<body style=…>`. Read with
//! html5ever's tokenizer (the first `body` start tag), written as one declaration list for the
//! frame's own `<body style>`, and scrubbed like any other style. The tokenizer stops at that
//! tag, and does not start when the markup has no `<body` in it to find.

use super::css::{self, Context};
use html5ever::TokenizerResult;
use html5ever::buffer_queue::BufferQueue;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    Tag, TagKind, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use std::cell::{Cell, RefCell};

struct Sink {
    body: RefCell<Option<Tag>>,
    /// Whether finding it pauses the tokenizer: not once `end` runs, which may not be paused.
    pause: Cell<bool>,
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        if let TagToken(tag) = token
            && tag.kind == TagKind::StartTag
            && &*tag.name == "body"
            && self.body.borrow().is_none()
        {
            *self.body.borrow_mut() = Some(tag);
            // Nothing after the first `<body>` is read: pause the tokenizer there, which hands
            // `feed` back at once.
            if self.pause.get() {
                return TokenSinkResult::Script(());
            }
        }
        TokenSinkResult::Continue
    }
}

/// A colour attribute's value when it is a plain colour: a `#` and hex digits, or a name.
/// Anything else (a `;`, a quote, a function) is not one, and is left out.
fn colour(value: &str) -> Option<&str> {
    let value = value.trim();
    let plain = match value.strip_prefix('#') {
        Some(hex) => {
            !hex.is_empty() && hex.len() <= 8 && hex.chars().all(|c| c.is_ascii_hexdigit())
        }
        None => !value.is_empty() && value.chars().all(|c| c.is_ascii_alphabetic()),
    };
    plain.then_some(value)
}

/// The first `<body>`'s `bgcolor`, `text` and `style`, as one scrubbed declaration list, or
/// `None` when it has none of them.
pub(super) fn body_style(html: &str) -> Option<String> {
    // A body start tag is `<` and the name `body` in any case, so markup without that has none.
    if !html
        .as_bytes()
        .windows(5)
        .any(|it| it.eq_ignore_ascii_case(b"<body"))
    {
        return None;
    }
    let mut tendril = StrTendril::new();
    tendril.push_slice(html);
    let queue = BufferQueue::default();
    queue.push_back(tendril);
    let tokenizer = Tokenizer::new(
        Sink {
            body: RefCell::new(None),
            pause: Cell::new(true),
        },
        TokenizerOpts::default(),
    );
    // Paused at the tag when there was one; otherwise read to the end.
    if let TokenizerResult::Done = tokenizer.feed(&queue) {
        tokenizer.sink.pause.set(false);
        tokenizer.end();
    }
    let tag = tokenizer.sink.body.borrow_mut().take()?;
    let mut declarations = Vec::new();
    for attribute in &tag.attrs {
        let value = attribute.value.as_ref();
        match &*attribute.name.local {
            "bgcolor" => {
                if let Some(colour) = colour(value) {
                    declarations.push(format!("background-color:{colour}"));
                }
            }
            "text" => {
                if let Some(colour) = colour(value) {
                    declarations.push(format!("color:{colour}"));
                }
            }
            _ => {}
        }
    }
    // The inline style last, so it wins over the attributes as it does in a browser.
    if let Some(style) = tag.attrs.iter().find(|a| &*a.name.local == "style") {
        let scrubbed = css::scrub(style.value.as_ref(), Context::Declarations);
        let trimmed = scrubbed.trim().trim_end_matches(';');
        if !trimmed.is_empty() {
            declarations.push(trimmed.to_owned());
        }
    }
    (!declarations.is_empty()).then(|| declarations.join(";"))
}

#[cfg(test)]
mod tests {
    use super::body_style;

    #[test]
    fn the_body_s_colours_and_style_are_kept_and_scrubbed() {
        const CASES: &[(&str, Option<&str>)] = &[
            ("<p>no body</p>", None),
            ("<html><body><p>x</p></body></html>", None),
            (
                "<body bgcolor=\"#f3f1ec\" text=\"#222\">",
                Some("background-color:#f3f1ec;color:#222"),
            ),
            ("<BODY BGCOLOR=white>", Some("background-color:white")),
            ("<body bgcolor=\"red;background:url(x)\">", None),
            (
                "<body style=\"margin:0;padding:0;background:url(https://t.test/x)\">",
                Some("margin:0;padding:0;background:none"),
            ),
            (
                "<body bgcolor=\"#000\" style=\"background-color:#111\">",
                Some("background-color:#000;background-color:#111"),
            ),
            (
                "<!-- <body bgcolor=red> --><body bgcolor=blue>",
                Some("background-color:blue"),
            ),
        ];
        for (html, want) in CASES {
            assert_eq!(body_style(html).as_deref(), *want, "{html}");
        }
    }
}
