//! A minimal, safe HTML writer.
//!
//! Every character of text is escaped. A link is written only from a [`SafeUrl`]. No
//! element carries a `style`, a class or an id: the reader's client styles the mail.

use mail_mime::SafeUrl;

use super::{ListTag, block_text, list_end};
use crate::editor::doc::{Check, Doc, Level, Mark, Node, Object, ParaKind, Run};
use crate::editor::text::runs_text;

/// HTML for `Draft.html`.
///
/// Consecutive list items share one list. A to-do is a bullet whose text starts with ☐ or ☑,
/// which is what a reader without to-dos shows, and what the paste path reads back.
pub fn to_html(doc: &Doc) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < doc.nodes.len() {
        match &doc.nodes[index] {
            Node::Para { kind, runs } => match ListTag::of(*kind) {
                Some(tag) => index = write_items(&mut out, &doc.nodes, index, tag),
                None => {
                    write_para(&mut out, *kind, runs);
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
    let (open, close) = match tag {
        ListTag::Numbered => ("<ol>", "</ol>"),
        ListTag::Bullet | ListTag::Todo => ("<ul>", "</ul>"),
    };
    out.push_str(open);
    for node in &nodes[start..end] {
        let Node::Para { kind, runs } = node else {
            continue;
        };
        // The item's text sits in a paragraph. A bare `<li>` makes `from_html` close the
        // text the moment an inline tag opens, so "say **hi**" would come back as two blocks.
        out.push_str("<li><p>");
        if let ParaKind::Todo(check) = kind {
            push_text(out, todo_glyph(*check), Breaks::Keep);
        }
        write_runs(out, runs);
        out.push_str("</p></li>");
    }
    out.push_str(close);
    end
}

fn write_para(out: &mut String, kind: ParaKind, runs: &[Run]) {
    match kind {
        ParaKind::Heading(level) => {
            let tag = match level {
                Level::One => "h1",
                Level::Two => "h2",
                Level::Three => "h3",
            };
            out.push_str(&format!("<{tag}>"));
            write_runs(out, runs);
            out.push_str(&format!("</{tag}>"));
        }
        ParaKind::Quote => {
            out.push_str("<blockquote><p>");
            write_runs(out, runs);
            out.push_str("</p></blockquote>");
        }
        ParaKind::Code => {
            out.push_str("<pre>");
            push_text(out, &runs_text(runs), Breaks::Keep);
            out.push_str("</pre>");
        }
        ParaKind::Paragraph | ParaKind::Bullet | ParaKind::Numbered | ParaKind::Todo(_) => {
            out.push_str("<p>");
            write_runs(out, runs);
            out.push_str("</p>");
        }
    }
}

fn write_runs(out: &mut String, runs: &[Run]) {
    for run in runs {
        write_run(out, run);
    }
}

/// One run: the link outermost, then the styles, in a fixed order.
fn write_run(out: &mut String, run: &Run) {
    if let Some(url) = &run.marks.link {
        out.push_str("<a href=\"");
        push_text(out, url.as_str(), Breaks::Keep);
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
            if let Some(url) = SafeUrl::parse(src.as_str()) {
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
