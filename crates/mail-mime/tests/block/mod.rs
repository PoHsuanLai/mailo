//! Shared sketch and walkers for the block tests.
//!
//! Each `block_*.rs` binary includes this file and uses a different subset.
#![allow(dead_code)]

use mail_domain::Inline;
use mail_mime::{
    Block, Dir, Document, Flowed, ImgSrc, ParsedPart, Reached, RemoteImages, SafeUrl,
    SanitizePolicy, Shape, Span, from_html, from_text, sanitize,
};

pub fn policy(images: RemoteImages) -> SanitizePolicy {
    SanitizePolicy {
        remote_images: images,
        version: SanitizePolicy::CURRENT.version,
    }
}

pub fn html(raw: &str) -> Document {
    let safe = sanitize(raw, policy(RemoteImages::Allowed));
    from_html(&safe, &[], RemoteImages::Blocked)
}

pub fn html_images(raw: &str, parts: &[ParsedPart], images: RemoteImages) -> Document {
    // Sanitize with images allowed so a remote `src` is still there for the
    // block parser to accept or reduce to a host.
    let safe = sanitize(raw, policy(RemoteImages::Allowed));
    from_html(&safe, parts, images)
}

pub fn text(raw: &str, flowed: Flowed) -> Document {
    from_text(raw, flowed)
}

pub fn part(cid: &str, mime: &str, bytes: &[u8]) -> ParsedPart {
    ParsedPart {
        name: "logo".to_owned(),
        mime: mime.to_owned(),
        bytes: bytes.to_vec(),
        inline: Inline::Embedded {
            cid: cid.to_owned(),
        },
        remote: None,
    }
}

/// A stable one-line-per-block sketch. Mapping tests compare this exactly.
pub fn sketch(doc: &Document) -> String {
    let mut out = String::new();
    let shape = match doc.shape {
        Shape::Letter => "letter",
        Shape::Layout => "layout",
        Shape::Machine => "machine",
    };
    let reached = match doc.reached {
        Reached::Nothing => "nothing",
        Reached::Depth => "depth",
        Reached::Nodes => "nodes",
        Reached::Blocks => "blocks",
        Reached::Text => "text",
    };
    let primary = match &doc.primary {
        Some(action) => format!("{} -> {}", action.label, action.url.as_str()),
        None => "-".to_owned(),
    };
    out.push_str(shape);
    out.push(' ');
    out.push_str(reached);
    out.push(' ');
    out.push_str(&primary);
    out.push('\n');
    sketch_blocks(&mut out, &doc.blocks, 0);
    out
}

fn sketch_blocks(out: &mut String, blocks: &[Block], indent: usize) {
    for block in blocks {
        pad(out, indent);
        match block {
            Block::Paragraph { spans, dir } => {
                out.push_str("p ");
                out.push_str(dir_name(*dir));
                out.push(' ');
                out.push_str(&spans_text(spans));
                out.push('\n');
            }
            Block::Heading { level, spans } => {
                out.push_str(&format!("h{level} {}\n", spans_text(spans)));
            }
            Block::List { ordered, items } => {
                out.push_str(if *ordered { "ol\n" } else { "ul\n" });
                for item in items {
                    pad(out, indent + 2);
                    out.push_str("item\n");
                    sketch_blocks(out, item, indent + 4);
                }
            }
            Block::Quote {
                attribution,
                blocks,
            } => {
                out.push_str("quote\n");
                if let Some(spans) = attribution {
                    pad(out, indent + 2);
                    out.push_str("@ ");
                    out.push_str(&spans_text(spans));
                    out.push('\n');
                }
                sketch_blocks(out, blocks, indent + 2);
            }
            Block::Code { lang, text } => {
                let lang = lang.as_deref().unwrap_or("-");
                let flat = text.replace('\n', "\\n");
                out.push_str(&format!("code {lang} {flat}\n"));
            }
            Block::Table { head, rows } => {
                out.push_str("table\n");
                if let Some(head) = head {
                    pad(out, indent + 2);
                    out.push_str("head ");
                    out.push_str(&row_text(head));
                    out.push('\n');
                }
                for row in rows {
                    pad(out, indent + 2);
                    out.push_str("row ");
                    out.push_str(&row_text(row));
                    out.push('\n');
                }
            }
            Block::Facts(pairs) => {
                out.push_str("facts\n");
                for (label, value) in pairs {
                    pad(out, indent + 2);
                    out.push_str(&spans_text(label));
                    out.push_str(" = ");
                    out.push_str(&spans_text(value));
                    out.push('\n');
                }
            }
            Block::Image {
                src,
                alt,
                width,
                height,
            } => {
                let wh = match (width, height) {
                    (Some(w), Some(h)) => format!(" {w}x{h}"),
                    _ => String::new(),
                };
                match src {
                    ImgSrc::Inline(uri) => {
                        let mime = uri
                            .as_str()
                            .strip_prefix("data:")
                            .and_then(|rest| rest.split(';').next())
                            .unwrap_or("?");
                        out.push_str(&format!("image inline {mime} {alt}{wh}\n"));
                    }
                    ImgSrc::Remote(url) => {
                        out.push_str(&format!("image remote {} {alt}{wh}\n", url.as_str()));
                    }
                    ImgSrc::Blocked { host } => {
                        out.push_str(&format!("image blocked {host} {alt}{wh}\n"));
                    }
                }
            }
            Block::Button { label, url } => {
                out.push_str(&format!("button {label} {}\n", url.as_str()));
            }
            Block::Signature(blocks) => {
                out.push_str("sig\n");
                sketch_blocks(out, blocks, indent + 2);
            }
            Block::Rule => out.push_str("rule\n"),
        }
    }
}

fn row_text(row: &[Vec<Span>]) -> String {
    row.iter()
        .map(|cell| spans_text(cell))
        .collect::<Vec<_>>()
        .join(" | ")
}

pub fn spans_text(spans: &[Span]) -> String {
    let mut out = String::new();
    write_spans(&mut out, spans);
    out
}

fn write_spans(out: &mut String, spans: &[Span]) {
    for span in spans {
        match span {
            Span::Text(text) => out.push_str(text),
            Span::Strong(inner) => {
                out.push('*');
                write_spans(out, inner);
                out.push('*');
            }
            Span::Emphasis(inner) => {
                out.push('_');
                write_spans(out, inner);
                out.push('_');
            }
            Span::Code(text) => {
                out.push('`');
                out.push_str(text);
                out.push('`');
            }
            Span::Link { url, spans } => {
                out.push('[');
                write_spans(out, spans);
                out.push_str("](");
                out.push_str(url.as_str());
                out.push(')');
            }
            Span::Break => out.push('⏎'),
        }
    }
}

fn pad(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push(' ');
    }
}

fn dir_name(dir: Dir) -> &'static str {
    match dir {
        Dir::Auto => "auto",
        Dir::Ltr => "ltr",
        Dir::Rtl => "rtl",
    }
}

/// Visible characters, with breaks and structure contributing nothing of their own.
pub fn plain(doc: &Document) -> String {
    let mut out = String::new();
    write_plain(&mut out, &doc.blocks);
    if let Some(action) = &doc.primary {
        out.push_str(&action.label);
    }
    out
}

fn write_plain(out: &mut String, blocks: &[Block]) {
    for block in blocks {
        match block {
            Block::Paragraph { spans, .. } | Block::Heading { spans, .. } => {
                write_plain_spans(out, spans);
            }
            Block::List { items, .. } => {
                for item in items {
                    write_plain(out, item);
                }
            }
            Block::Quote {
                attribution,
                blocks,
            } => {
                if let Some(spans) = attribution {
                    write_plain_spans(out, spans);
                }
                write_plain(out, blocks);
            }
            Block::Code { text, .. } => out.push_str(text),
            Block::Table { head, rows } => {
                if let Some(head) = head {
                    for cell in head {
                        write_plain_spans(out, cell);
                    }
                }
                for row in rows {
                    for cell in row {
                        write_plain_spans(out, cell);
                    }
                }
            }
            Block::Facts(pairs) => {
                for (label, value) in pairs {
                    write_plain_spans(out, label);
                    write_plain_spans(out, value);
                }
            }
            Block::Button { label, .. } => out.push_str(label),
            Block::Signature(blocks) => write_plain(out, blocks),
            Block::Image { .. } | Block::Rule => {}
        }
    }
}

fn write_plain_spans(out: &mut String, spans: &[Span]) {
    for span in spans {
        match span {
            Span::Text(text) | Span::Code(text) => out.push_str(text),
            Span::Break => {}
            Span::Strong(inner) | Span::Emphasis(inner) | Span::Link { spans: inner, .. } => {
                write_plain_spans(out, inner);
            }
        }
    }
}

pub fn urls(doc: &Document) -> Vec<SafeUrl> {
    let mut out = Vec::new();
    if let Some(action) = &doc.primary {
        out.push(action.url.clone());
    }
    collect_urls(&doc.blocks, &mut out);
    out
}

fn collect_urls(blocks: &[Block], out: &mut Vec<SafeUrl>) {
    for block in blocks {
        match block {
            Block::Paragraph { spans, .. } | Block::Heading { spans, .. } => {
                collect_span_urls(spans, out);
            }
            Block::List { items, .. } => {
                for item in items {
                    collect_urls(item, out);
                }
            }
            Block::Quote {
                attribution,
                blocks,
            } => {
                if let Some(spans) = attribution {
                    collect_span_urls(spans, out);
                }
                collect_urls(blocks, out);
            }
            Block::Table { head, rows } => {
                if let Some(head) = head {
                    for cell in head {
                        collect_span_urls(cell, out);
                    }
                }
                for row in rows {
                    for cell in row {
                        collect_span_urls(cell, out);
                    }
                }
            }
            Block::Facts(pairs) => {
                for (label, value) in pairs {
                    collect_span_urls(label, out);
                    collect_span_urls(value, out);
                }
            }
            Block::Image { src, .. } => {
                if let ImgSrc::Remote(url) = src {
                    out.push(url.clone());
                }
            }
            Block::Button { url, .. } => out.push(url.clone()),
            Block::Signature(blocks) => collect_urls(blocks, out),
            Block::Code { .. } | Block::Rule => {}
        }
    }
}

fn collect_span_urls(spans: &[Span], out: &mut Vec<SafeUrl>) {
    for span in spans {
        match span {
            Span::Link { url, spans } => {
                out.push(url.clone());
                collect_span_urls(spans, out);
            }
            Span::Strong(inner) | Span::Emphasis(inner) => collect_span_urls(inner, out),
            Span::Text(_) | Span::Code(_) | Span::Break => {}
        }
    }
}

pub fn images(doc: &Document) -> Vec<ImgSrc> {
    let mut out = Vec::new();
    collect_images(&doc.blocks, &mut out);
    out
}

fn collect_images(blocks: &[Block], out: &mut Vec<ImgSrc>) {
    for block in blocks {
        match block {
            Block::Image { src, .. } => out.push(src.clone()),
            Block::List { items, .. } => {
                for item in items {
                    collect_images(item, out);
                }
            }
            Block::Quote { blocks, .. } | Block::Signature(blocks) => collect_images(blocks, out),
            _ => {}
        }
    }
}

pub fn norm(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in text.chars() {
        if matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') {
            continue;
        }
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(ch);
    }
    out
}
