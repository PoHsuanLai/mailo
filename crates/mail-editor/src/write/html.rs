//! A minimal, safe HTML writer.
//!
//! Every character of text is escaped. A link is written only from a [`SafeUrl`]. No element
//! carries a class or an id, and a `style` only says what the composer set, each from its fixed
//! few ([`ParaStyle`] and [`RunStyle`]): the reader's client styles the rest of the mail.

use mail_mime::{Align, ParaStyle, RunStyle, SafeUrl};

use super::{ListTag, block_text, list_end, same_list, written_depths};
use crate::doc::{Check, Doc, Layout, Level, Mark, Node, Object, ParaKind, Run};
use crate::text::runs_text;

/// HTML for `Draft.html`.
///
/// Consecutive list items share one list, a deeper item a list inside the item before it.
/// Consecutive quotes share one quote, a deeper level a quote inside it. A to-do is a bullet
/// whose text starts with ☐ or ☑, which is what a reader without to-dos shows, and what the
/// paste path reads back.
pub fn to_html(doc: &Doc) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < doc.nodes.len() {
        match &doc.nodes[index] {
            Node::Para { kind, runs, layout } => match ListTag::of(*kind) {
                Some(tag) => index = write_items(&mut out, &doc.nodes, index, tag),
                None if *kind == ParaKind::Quote => {
                    index = write_quotes(&mut out, &doc.nodes, index);
                }
                None => {
                    write_para(&mut out, *kind, runs, *layout);
                    index += 1;
                }
            },
            Node::Object(object) => {
                write_object(&mut out, object);
                index += 1;
            }
        }
    }
    out
}

/// The glyph a to-do item's text starts with in HTML.
pub(crate) fn todo_glyph(check: Check) -> &'static str {
    match check {
        Check::Open => "☐ ",
        Check::Done => "☑ ",
    }
}

fn write_items(out: &mut String, nodes: &[Node], start: usize, tag: ListTag) -> usize {
    let end = list_end(nodes, start, tag);
    let items = &nodes[start..end];
    // The lists open around the item being written, outermost first. Each holds an open `<li>`.
    let mut open: Vec<ListTag> = Vec::new();
    for (node, depth) in items.iter().zip(written_depths(items)) {
        let Node::Para { kind, runs, layout } = node else {
            continue;
        };
        let Some(tag) = ListTag::of(*kind) else {
            continue;
        };
        while open.len() > depth + 1 {
            close_list(out, open.pop());
        }
        match open.last() {
            Some(last) if open.len() == depth + 1 && same_list(*last, tag) => {
                out.push_str("</li>");
            }
            Some(_) if open.len() == depth + 1 => {
                close_list(out, open.pop());
                open_list(out, tag, &mut open);
            }
            _ => open_list(out, tag, &mut open),
        }
        // The item's text sits in a paragraph. A bare `<li>` makes `from_html` close the
        // text the moment an inline tag opens, so "say **hi**" would come back as two blocks.
        out.push_str("<li>");
        open_tag(out, "p", aligned(layout.align));
        if let ParaKind::Todo(check) = kind {
            push_text(out, todo_glyph(*check), Breaks::Keep);
        }
        write_runs(out, runs);
        out.push_str("</p>");
    }
    while !open.is_empty() {
        close_list(out, open.pop());
    }
    end
}

fn open_list(out: &mut String, tag: ListTag, open: &mut Vec<ListTag>) {
    out.push_str(match tag {
        ListTag::Numbered => "<ol>",
        ListTag::Bullet | ListTag::Todo => "<ul>",
    });
    open.push(tag);
}

/// Close the item left open in `tag`'s list, and the list.
fn close_list(out: &mut String, tag: Option<ListTag>) {
    out.push_str(match tag {
        Some(ListTag::Numbered) => "</li></ol>",
        Some(ListTag::Bullet | ListTag::Todo) => "</li></ul>",
        None => "",
    });
}

/// The run of quotes from `start`, one `blockquote` per level: a quote at depth 1 sits in a
/// quote inside the quote. Returns the index past the run.
fn write_quotes(out: &mut String, nodes: &[Node], start: usize) -> usize {
    let mut levels = 0;
    let mut index = start;
    while let Some(Node::Para {
        kind: ParaKind::Quote,
        runs,
        layout,
    }) = nodes.get(index)
    {
        let want = usize::from(layout.depth) + 1;
        while levels > want {
            out.push_str("</blockquote>");
            levels -= 1;
        }
        while levels < want {
            out.push_str("<blockquote>");
            levels += 1;
        }
        open_tag(out, "p", aligned(layout.align));
        write_runs(out, runs);
        out.push_str("</p>");
        index += 1;
    }
    for _ in 0..levels {
        out.push_str("</blockquote>");
    }
    index
}

fn write_para(out: &mut String, kind: ParaKind, runs: &[Run], layout: Layout) {
    let style = ParaStyle {
        align: layout.align,
        indent: layout.depth,
    };
    match kind {
        ParaKind::Heading(level) => {
            let tag = match level {
                Level::One => "h1",
                Level::Two => "h2",
                Level::Three => "h3",
            };
            open_tag(out, tag, style);
            write_runs(out, runs);
            out.push_str(&format!("</{tag}>"));
        }
        ParaKind::Code => {
            out.push_str("<pre>");
            push_text(out, &runs_text(runs), Breaks::Keep);
            out.push_str("</pre>");
        }
        ParaKind::Paragraph
        | ParaKind::Quote
        | ParaKind::Bullet
        | ParaKind::Numbered
        | ParaKind::Todo(_) => {
            open_tag(out, "p", style);
            write_runs(out, runs);
            out.push_str("</p>");
        }
    }
}

/// A paragraph's style with its alignment alone: an item's and a quote's depth is the lists or
/// quotes around them, not an indent.
fn aligned(align: Align) -> ParaStyle {
    ParaStyle { align, indent: 0 }
}

/// `<tag>`, with `style`'s attribute when it says anything.
fn open_tag(out: &mut String, tag: &str, style: ParaStyle) {
    out.push('<');
    out.push_str(tag);
    if let Some(css) = style.css() {
        out.push_str(" style=\"");
        out.push_str(&css);
        out.push('"');
    }
    out.push('>');
}

fn write_runs(out: &mut String, runs: &[Run]) {
    for run in runs {
        write_run(out, run);
    }
}

/// One run: the link outermost, then its size and colour, then the styles, in a fixed order.
fn write_run(out: &mut String, run: &Run) {
    if let Some(url) = &run.marks.link {
        out.push_str("<a href=\"");
        push_text(out, url.as_str(), Breaks::Keep);
        out.push_str("\">");
    }
    let styled = RunStyle {
        size: run.marks.size,
        color: run.marks.color,
    }
    .css();
    if let Some(css) = &styled {
        out.push_str("<span style=\"");
        out.push_str(css);
        out.push_str("\">");
    }
    let tags: Vec<&str> = [
        (Mark::Bold, "strong"),
        (Mark::Italic, "em"),
        (Mark::Underline, "u"),
        (Mark::Strike, "s"),
        (Mark::Code, "code"),
    ]
    .into_iter()
    .filter(|(mark, _)| run.marks.has(*mark))
    .map(|(_, tag)| tag)
    .collect();
    for tag in &tags {
        out.push_str(&format!("<{tag}>"));
    }
    push_text(out, &run.text, Breaks::Br);
    for tag in tags.iter().rev() {
        out.push_str(&format!("</{tag}>"));
    }
    if styled.is_some() {
        out.push_str("</span>");
    }
    if run.marks.link.is_some() {
        out.push_str("</a>");
    }
}

fn write_object(out: &mut String, object: &Object) {
    match object {
        Object::Divider => out.push_str("<hr>"),
        Object::Image { src, alt } => {
            out.push_str("<img alt=\"");
            push_text(out, alt, Breaks::Keep);
            out.push('"');
            if let Some(cid) = src.cid() {
                // The draft's own part, which the message carries beside the HTML.
                out.push_str(" src=\"cid:");
                push_text(out, cid, Breaks::Keep);
                out.push('"');
            } else if let Ok(url) = src.as_str().parse::<SafeUrl>() {
                out.push_str(" src=\"");
                push_text(out, url.as_str(), Breaks::Keep);
                out.push('"');
            }
            out.push('>');
        }
        Object::Table(table) => {
            out.push_str("<table>");
            for row in &table.rows {
                out.push_str("<tr>");
                for cell in row {
                    out.push_str("<td>");
                    push_text(out, cell, Breaks::Br);
                    out.push_str("</td>");
                }
                out.push_str("</tr>");
            }
            out.push_str("</table>");
        }
        Object::Signature => out.push_str("<p>-- </p>"),
        // The file travels as a MIME part. The body does not name it again.
        Object::Attachment(_) => {}
        Object::QuotedMessage { who, when, body } => {
            out.push_str("<blockquote><p>");
            push_text(out, who, Breaks::Keep);
            out.push_str(", ");
            push_text(out, when, Breaks::Keep);
            out.push_str("</p>");
            for text in body.iter().map(block_text).filter(|text| !text.is_empty()) {
                out.push_str("<p>");
                push_text(out, &text, Breaks::Br);
                out.push_str("</p>");
            }
            out.push_str("</blockquote>");
        }
    }
}

/// Whether a newline in text becomes `<br>`. Inside `<pre>` and attributes it stays.
#[derive(Clone, Copy)]
enum Breaks {
    Br,
    Keep,
}

fn push_text(out: &mut String, text: &str, breaks: Breaks) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' if matches!(breaks, Breaks::Br) => out.push_str("<br>"),
            other => out.push(other),
        }
    }
}
