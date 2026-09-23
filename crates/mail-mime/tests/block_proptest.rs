//! The two properties where one meaning has two implementations.
//!
//! Text in the blocks equals the text an independent tokenizer walk sees.
//! A mapping that drops a subtree fails that, which is the failure that
//! loses mail. Every URL in the output parses back to itself.

#[path = "block/mod.rs"]
mod support;

use html5ever::buffer_queue::BufferQueue;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};
use mail_mime::{SafeUrl, from_html, sanitize};
use proptest::prelude::*;
use std::cell::RefCell;

proptest! {
    #[test]
    fn block_text_matches_an_independent_walk(fragment in arb_fragment()) {
        let safe = sanitize(&fragment, support::policy(mail_mime::RemoteImages::Blocked));
        let doc = from_html(&safe, &[], mail_mime::RemoteImages::Blocked);
        let from_blocks = support::norm(&support::plain(&doc));
        let from_tokens = support::norm(&token_text(safe.as_str()));
        prop_assert_eq!(from_blocks, from_tokens, "fragment was {}", fragment);
    }

    #[test]
    fn every_url_in_the_output_parses_back_to_itself(fragment in arb_fragment()) {
        let safe = sanitize(&fragment, support::policy(mail_mime::RemoteImages::Allowed));
        let doc = from_html(&safe, &[], mail_mime::RemoteImages::Allowed);
        for url in support::urls(&doc) {
            prop_assert_eq!(SafeUrl::parse(url.as_str()), Some(url.clone()));
        }
    }
}

fn arb_fragment() -> impl Strategy<Value = String> {
    let word = prop::collection::vec(prop::char::range('a', 'z'), 1..=8)
        .prop_map(|chars| chars.into_iter().collect::<String>());
    let piece = word.prop_flat_map(|word| {
        prop_oneof![
            Just(format!("<p>{word}</p>")),
            Just(format!("<div><b>{word}</b></div>")),
            Just(format!("<h2>{word}</h2>")),
            Just(format!("<ul><li>{word}</li></ul>")),
            Just(format!("<blockquote><p>{word}</p></blockquote>")),
            Just(format!("<table><tr><td>{word}</td></tr></table>")),
            Just(format!("<pre>{word}</pre>")),
            Just(format!("<main><p>{word}</p></main>")),
            Just(format!(
                r#"<p>see <a href="https://example.test/{word}">{word}</a></p>"#
            )),
            Just(format!("<p>{word}<br>{word}</p>")),
        ]
    });
    prop::collection::vec(piece, 0..=6).prop_map(|parts| parts.concat())
}

fn token_text(html: &str) -> String {
    let mut tendril = StrTendril::new();
    tendril.push_slice(html);
    let queue = BufferQueue::default();
    queue.push_back(tendril);
    let sink = TextSink {
        text: RefCell::new(String::new()),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    tokenizer.sink.text.borrow().clone()
}

struct TextSink {
    text: RefCell<String>,
}

impl TokenSink for TextSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        if let Token::CharacterTokens(text) = token {
            self.text.borrow_mut().push_str(&text);
        }
        TokenSinkResult::Continue
    }
}
