//! A data table, with its amount columns set to the right.
//!
//! Whether a column holds amounts is decided from its cells, because the
//! sender's `align` did not survive into the blocks.

use super::spans::spans;
use dioxus::prelude::*;
use mail_mime::Span;

pub(super) fn table(path: &str, head: &Option<Vec<Vec<Span>>>, rows: &[Vec<Vec<Span>>]) -> Element {
    let numeric = numeric_columns(rows);
    rsx! {
        div { key: "{path}", class: "b b-table",
            table {
                if let Some(head) = head {
                    thead {
                        tr {
                            for (index, cell) in head.iter().enumerate() {
                                th { key: "{index}", {spans(cell)} }
                            }
                        }
                    }
                }
                tbody {
                    for (row_index, row) in rows.iter().enumerate() {
                        tr { key: "{row_index}",
                            for (index, cell) in row.iter().enumerate() {
                                td {
                                    key: "{index}",
                                    class: if numeric.get(index).copied().unwrap_or(false) { "num" } else { "" },
                                    {spans(cell)}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A column of amounts. Empty cells do not count against it; a word does.
fn numeric_columns(rows: &[Vec<Vec<Span>>]) -> Vec<bool> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = vec![false; width];
    for (column, flag) in out.iter_mut().enumerate() {
        let mut saw_digit = false;
        let mut words = false;
        for row in rows {
            let Some(cell) = row.get(column) else {
                continue;
            };
            let text = span_text(cell);
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            if looks_numeric(text) {
                saw_digit = true;
            } else {
                words = true;
            }
        }
        *flag = saw_digit && !words;
    }
    out
}

fn looks_numeric(text: &str) -> bool {
    let text = text.trim_start_matches(|ch: char| {
        matches!(
            ch,
            '€' | '$' | '£' | '¥' | '+' | '-' | '−' | '–' | '×' | 'x' | ' '
        )
    });
    let text = text.trim();
    let mut digit = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digit = true;
            continue;
        }
        if !matches!(ch, ',' | '.' | '%' | ' ' | '\u{00a0}' | '\u{202f}') {
            return false;
        }
    }
    digit
}

fn span_text(items: &[Span]) -> String {
    let mut out = String::new();
    fn walk(out: &mut String, items: &[Span]) {
        for span in items {
            match span {
                Span::Text(text) | Span::Code(text) => out.push_str(text),
                Span::Break => out.push(' '),
                Span::Strong(inner) | Span::Emphasis(inner) | Span::Link { spans: inner, .. } => {
                    walk(out, inner);
                }
            }
        }
    }
    walk(&mut out, items);
    out
}
