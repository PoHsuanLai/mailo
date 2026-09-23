//! Ordering and verdict over the fixtures. Not the absolute scores.

#[path = "block/mod.rs"]
mod support;

use mail_mime::{Block, Shape, Span};
use support::{html, sketch};

#[test]
fn a_letter_is_lighter_than_a_newsletter_and_neither_is_the_other() {
    let letter = html(include_str!("fixtures/block/letter.html"));
    let news = html(include_str!("fixtures/block/newsletter.html"));
    assert!(
        letter.heaviness() < news.heaviness(),
        "letter {} newsletter {}",
        letter.heaviness(),
        news.heaviness()
    );
    assert_eq!(letter.shape, Shape::Letter, "{}", sketch(&letter));
    assert_eq!(news.shape, Shape::Layout, "{}", sketch(&news));
    assert!(
        news.primary.is_none(),
        "a newsletter is not a receipt: {}",
        sketch(&news)
    );
    assert!(
        news.blocks
            .iter()
            .any(|block| matches!(block, Block::Button { .. })),
        "the issue link should stay in the body: {}",
        sketch(&news)
    );
}

#[test]
fn a_letter_is_not_machine_mail() {
    let letter = html(include_str!("fixtures/block/letter.html"));
    assert_eq!(letter.shape, Shape::Letter);
    assert!(letter.primary.is_none());
    assert!(
        !letter
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Facts(_))),
        "{}",
        sketch(&letter)
    );
}

#[test]
fn machine_mail_promotes_one_action() {
    let doc = html(include_str!("fixtures/block/receipt.html"));
    assert_eq!(doc.shape, Shape::Machine, "{}", sketch(&doc));
    let got = sketch(&doc);
    let primary = doc.primary.as_ref().unwrap_or_else(|| panic!("{got}"));
    assert_eq!(primary.label.to_ascii_lowercase(), "track your parcel");
    assert_eq!(primary.url.as_str(), "https://shop.example/track/48213");
    assert!(
        has_order_number(&doc.blocks),
        "no facts carried the order number: {}",
        sketch(&doc)
    );
    assert!(
        !contains_button(&doc.blocks, "https://shop.example/track/48213"),
        "the button stayed in the body: {}",
        sketch(&doc)
    );
}

fn has_order_number(blocks: &[Block]) -> bool {
    blocks.iter().any(|block| match block {
        Block::Facts(pairs) => pairs.iter().any(|(label, _)| {
            support::spans_text(label)
                .to_ascii_lowercase()
                .contains("order number")
        }),
        Block::Quote { blocks, .. } | Block::Signature(blocks) => has_order_number(blocks),
        Block::List { items, .. } => items.iter().any(|item| has_order_number(item)),
        _ => false,
    })
}

fn contains_button(blocks: &[Block], url: &str) -> bool {
    blocks.iter().any(|block| match block {
        Block::Button { url: button, .. } => button.as_str() == url,
        Block::Paragraph { spans, .. } => spans.iter().any(|span| match span {
            Span::Link { url: link, .. } => link.as_str() == url,
            _ => false,
        }),
        Block::Quote { blocks, .. } | Block::Signature(blocks) => contains_button(blocks, url),
        Block::List { items, .. } => items.iter().any(|item| contains_button(item, url)),
        _ => false,
    })
}
