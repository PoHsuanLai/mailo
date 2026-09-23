//! Machine mail: a layout-heavy body with a footer, a label/value table,
//! or one short link that is the whole of its block — at least two of those.
//!
//! It is a count of signals, the same idea as heaviness. One signal is a
//! letter that happens to link somewhere, or a newsletter with a button.
//! Two is a receipt. The dominant link becomes [`Action`] and leaves the
//! body; two-column label/value tables become [`Block::Facts`].

use super::kind::{Action, Block, Shape, Span, span_text, word_count};
use super::url::SafeUrl;

/// Buttons for lone short links, then the machine verdict.
///
/// Button promotion runs for every shape: a newsletter's "read the issue"
/// link is a button even when the body is not a receipt. Only the machine
/// verdict lifts one button out of the body.
pub(crate) fn finish(blocks: &mut Vec<Block>, heavy: bool) -> (Shape, Option<Action>) {
    promote_buttons(blocks);
    if !heavy {
        return (Shape::Letter, None);
    }
    let footer = footer_in_tail(blocks);
    let facts = count_facts(blocks) > 0;
    let cta = has_cta(blocks);
    let signals = u8::from(footer) + u8::from(facts) + u8::from(cta);
    if signals < 2 {
        return (Shape::Layout, None);
    }
    let primary = take_cta(blocks);
    convert_facts(blocks);
    (Shape::Machine, primary)
}

fn promote_buttons(blocks: &mut [Block]) {
    for block in blocks.iter_mut() {
        match block {
            Block::Paragraph { spans, .. } => {
                if let Some((label, url)) = lone_link(spans)
                    && (1..=5).contains(&word_count(&label))
                {
                    *block = Block::Button { label, url };
                }
            }
            Block::Quote { blocks, .. } | Block::Signature(blocks) => promote_buttons(blocks),
            Block::List { items, .. } => {
                for item in items {
                    promote_buttons(item);
                }
            }
            _ => {}
        }
    }
}

fn has_cta(blocks: &[Block]) -> bool {
    blocks.iter().any(|block| match block {
        Block::Button { label, .. } => is_cta(label),
        Block::Quote { blocks, .. } | Block::Signature(blocks) => has_cta(blocks),
        Block::List { items, .. } => items.iter().any(|item| has_cta(item)),
        Block::Table { head, rows } => {
            head.as_ref()
                .is_some_and(|head| head.iter().any(|cell| cell_is_cta(cell)))
                || rows
                    .iter()
                    .any(|row| row.iter().any(|cell| cell_is_cta(cell)))
        }
        _ => false,
    })
}

fn cell_is_cta(spans: &[Span]) -> bool {
    lone_link(spans).is_some_and(|(label, _)| is_cta(&label))
}

fn take_cta(blocks: &mut Vec<Block>) -> Option<Action> {
    if let Some(index) = blocks.iter().position(|block| match block {
        Block::Button { label, .. } => is_cta(label),
        _ => false,
    }) {
        return match blocks.remove(index) {
            Block::Button { label, url } => Some(Action { label, url }),
            other => {
                blocks.insert(index, other);
                None
            }
        };
    }
    for block in blocks.iter_mut() {
        let found = match block {
            Block::Quote { blocks, .. } | Block::Signature(blocks) => take_cta(blocks),
            Block::List { items, .. } => {
                let mut found = None;
                for item in items {
                    found = take_cta(item);
                    if found.is_some() {
                        break;
                    }
                }
                found
            }
            Block::Table { head, rows } => take_cta_from_table(head, rows),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn take_cta_from_table(
    head: &mut Option<Vec<Vec<Span>>>,
    rows: &mut [Vec<Vec<Span>>],
) -> Option<Action> {
    if let Some(head) = head
        && let Some(action) = take_cta_from_cells(head)
    {
        return Some(action);
    }
    for row in rows {
        if let Some(action) = take_cta_from_cells(row) {
            return Some(action);
        }
    }
    None
}

fn take_cta_from_cells(cells: &mut [Vec<Span>]) -> Option<Action> {
    for cell in cells {
        if let Some((label, url)) = lone_link(cell)
            && is_cta(&label)
        {
            cell.clear();
            return Some(Action { label, url });
        }
    }
    None
}

fn convert_facts(blocks: &mut [Block]) {
    for block in blocks.iter_mut() {
        match block {
            Block::Table { head: None, rows } => {
                if let Some(pairs) = as_facts(rows) {
                    *block = Block::Facts(pairs);
                }
            }
            Block::Quote { blocks, .. } | Block::Signature(blocks) => convert_facts(blocks),
            Block::List { items, .. } => {
                for item in items {
                    convert_facts(item);
                }
            }
            _ => {}
        }
    }
}

fn count_facts(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            Block::Table { head: None, rows } if as_facts(rows).is_some() => 1,
            Block::Quote { blocks, .. } | Block::Signature(blocks) => count_facts(blocks),
            Block::List { items, .. } => items.iter().map(|item| count_facts(item)).sum(),
            _ => 0,
        })
        .sum()
}

/// A two-column table of short labels and short values.
fn as_facts(rows: &[Vec<Vec<Span>>]) -> Option<Vec<(Vec<Span>, Vec<Span>)>> {
    if rows.len() < 2 {
        return None;
    }
    let mut pairs = Vec::with_capacity(rows.len());
    for row in rows {
        let [label, value] = row.as_slice() else {
            return None;
        };
        let label_text = span_text(label);
        let value_text = span_text(value);
        if label_text.trim().is_empty() {
            return None;
        }
        if word_count(label_text.trim()) > 6 || label_text.chars().count() > 40 {
            return None;
        }
        if value_text.chars().count() > 80 {
            return None;
        }
        pairs.push((label.clone(), value.clone()));
    }
    Some(pairs)
}

fn lone_link(spans: &[Span]) -> Option<(String, SafeUrl)> {
    let mut found: Option<(String, SafeUrl)> = None;
    let mut extra = false;
    inspect(spans, &mut found, &mut extra, false);
    if extra { None } else { found }
}

fn inspect(spans: &[Span], found: &mut Option<(String, SafeUrl)>, extra: &mut bool, in_link: bool) {
    for span in spans {
        match span {
            Span::Break => {}
            Span::Text(text) | Span::Code(text) => {
                if in_link || text.trim().is_empty() {
                    continue;
                }
                *extra = true;
            }
            Span::Strong(inner) | Span::Emphasis(inner) => inspect(inner, found, extra, in_link),
            Span::Link { url, spans } => {
                if found.is_some() {
                    *extra = true;
                }
                *found = Some((span_text(spans), url.clone()));
                inspect(spans, found, extra, true);
            }
        }
    }
}

fn is_cta(label: &str) -> bool {
    let words = word_count(label);
    (1..=5).contains(&words) && !is_footer_label(label)
}

fn is_footer_label(label: &str) -> bool {
    FOOTER.iter().any(|phrase| contains_phrase(label, phrase))
}

fn footer_in_tail(blocks: &[Block]) -> bool {
    let start = blocks.len().saturating_sub(3);
    let mut text = String::new();
    for block in &blocks[start..] {
        push_text(&mut text, block);
        text.push('\n');
    }
    FOOTER.iter().any(|phrase| contains_phrase(&text, phrase))
}

fn push_text(out: &mut String, block: &Block) {
    match block {
        Block::Heading { spans, .. } | Block::Paragraph { spans, .. } => {
            out.push_str(&span_text(spans));
        }
        Block::List { items, .. } => {
            for item in items {
                for child in item {
                    push_text(out, child);
                }
            }
        }
        Block::Quote {
            attribution,
            blocks,
        } => {
            if let Some(spans) = attribution {
                out.push_str(&span_text(spans));
            }
            for child in blocks {
                push_text(out, child);
            }
        }
        Block::Code { text, .. } => out.push_str(text),
        Block::Table { head, rows } => {
            if let Some(head) = head {
                for cell in head {
                    out.push_str(&span_text(cell));
                    out.push(' ');
                }
            }
            for row in rows {
                for cell in row {
                    out.push_str(&span_text(cell));
                    out.push(' ');
                }
            }
        }
        Block::Facts(pairs) => {
            for (label, value) in pairs {
                out.push_str(&span_text(label));
                out.push(' ');
                out.push_str(&span_text(value));
                out.push(' ');
            }
        }
        Block::Button { label, .. } => out.push_str(label),
        Block::Signature(blocks) => {
            for child in blocks {
                push_text(out, child);
            }
        }
        Block::Image { alt, .. } => out.push_str(alt),
        Block::Rule => {}
    }
}

/// Sender-invariant footer words, in English, French, German, Spanish,
/// Portuguese, and Chinese. Matched as phrases on word boundaries, not as
/// substrings of a longer Latin token.
const FOOTER: &[&str] = &[
    "unsubscribe",
    "you received this",
    "privacy",
    "désabonner",
    "desabonner",
    "vous recevez",
    "confidentialité",
    "confidentialite",
    "abmelden",
    "abbestellen",
    "sie erhalten",
    "datenschutz",
    "darse de baja",
    "cancelar suscripción",
    "cancelar suscripcion",
    "privacidad",
    "descadastrar",
    "cancelar inscrição",
    "cancelar inscricao",
    "você recebeu",
    "voce recebeu",
    "退订",
    "您收到",
    "隐私",
];

fn contains_phrase(hay: &str, phrase: &str) -> bool {
    let hay = hay.to_lowercase();
    let phrase = phrase.to_lowercase();
    if phrase
        .chars()
        .any(|ch| matches!(ch, '\u{4E00}'..='\u{9FFF}'))
    {
        return hay.contains(&phrase);
    }
    let mut from = 0;
    while let Some(rel) = hay.get(from..).and_then(|rest| rest.find(&phrase)) {
        let at = from + rel;
        let end = at + phrase.len();
        let before = hay[..at].chars().next_back();
        let after = hay.get(end..).and_then(|rest| rest.chars().next());
        let left = before.is_none_or(|ch| !ch.is_alphanumeric());
        let right = after.is_none_or(|ch| !ch.is_alphanumeric());
        if left && right {
            return true;
        }
        let next = at + phrase.len().max(1);
        if next <= from {
            break;
        }
        from = next;
    }
    false
}
