//! Which tag becomes which frame. A name with no role is transparent: the day
//! ammonia's allowlist grows, that tag's text is still mail.

use crate::block::kind::{Block, Dir, Span};
use crate::block::limits::{Limits, Reached};
use crate::block::url::SafeUrl;
use html5ever::tokenizer::Tag;

pub(super) fn push_chars(
    buf: &mut String,
    text: &str,
    raw: bool,
    mut sticky: bool,
    text_bytes: &mut usize,
    reached: &mut Reached,
) {
    for ch in text.chars() {
        if *text_bytes >= Limits::MAX_TEXT || buf.len() >= Limits::MAX_RUN {
            if *reached == Reached::Nothing {
                *reached = Reached::Text;
            }
            return;
        }
        if raw {
            buf.push(ch);
            *text_bytes += ch.len_utf8();
            continue;
        }
        if ch.is_whitespace() {
            // A break already ended the previous run, so the space after it
            // still separates words. Dropping it would lose text the tokenizer saw.
            if buf.is_empty() && sticky {
                buf.push(' ');
                *text_bytes += 1;
                sticky = false;
            } else if !buf.is_empty() && !buf.ends_with(' ') {
                buf.push(' ');
                *text_bytes += 1;
            }
        } else {
            buf.push(ch);
            *text_bytes += ch.len_utf8();
            sticky = false;
        }
    }
}

pub(super) fn flush_buf(frame: &mut Frame) {
    if frame.raw {
        return;
    }
    let text = std::mem::take(&mut frame.buf);
    if text.is_empty() {
        return;
    }
    if text.chars().all(|ch| ch == ' ') && frame.spans.is_empty() {
        return;
    }
    frame.spans.push(Span::Text(text));
}

pub(super) fn trim_spans(spans: &mut Vec<Span>) {
    if spans.len() == 1 {
        if let Span::Text(text) = &mut spans[0] {
            *text = text.trim().to_owned();
        }
    } else if !spans.is_empty() {
        if let Span::Text(text) = &mut spans[0] {
            *text = text.trim_start().to_owned();
        }
        let last = spans.len() - 1;
        if let Span::Text(text) = &mut spans[last] {
            *text = text.trim_end().to_owned();
        }
    }
    spans.retain(|span| !matches!(span, Span::Text(text) if text.is_empty()));
}

pub(super) fn dir_attr(tag: &Tag) -> Option<Dir> {
    let value = attr(tag, "dir")?;
    match value.trim().to_ascii_lowercase().as_str() {
        "ltr" => Some(Dir::Ltr),
        "rtl" => Some(Dir::Rtl),
        "auto" => Some(Dir::Auto),
        _ => None,
    }
}

pub(super) fn attr<'a>(tag: &'a Tag, name: &str) -> Option<&'a str> {
    tag.attrs
        .iter()
        .find(|attribute| &*attribute.name.local == name)
        .map(|attribute| &*attribute.value)
}

pub(super) fn presentational(tag: &Tag) -> bool {
    tag.attrs.iter().any(|attribute| {
        matches!(
            &*attribute.name.local,
            "align"
                | "width"
                | "height"
                | "bgcolor"
                | "border"
                | "cellpadding"
                | "cellspacing"
                | "valign"
                | "size"
                | "char"
                | "charoff"
                | "nowrap"
                | "background"
                | "color"
        )
    })
}

pub(super) fn parse_px(value: &str) -> Option<u32> {
    let value = value.trim().trim_end_matches("px").trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub(super) fn code_lang(value: &str) -> Option<String> {
    let value = value.trim();
    if !(2..=16).contains(&value.len()) || value.contains('-') || value.len() == 2 {
        return None;
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '+')
    {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

pub(super) fn is_void(name: &str) -> bool {
    matches!(name, "area" | "br" | "col" | "hr" | "img" | "wbr")
}

/// Tags with an explicit role. Kept in lockstep with [`MAPPED`].
pub(super) fn open_as(name: &str) -> Option<()> {
    frame_kind(name).map(|_| ()).or_else(|| {
        is_void(name)
            .then_some(())
            .or_else(|| matches!(name, "thead" | "tbody" | "colgroup").then_some(()))
    })
}

pub(super) fn frame_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "p" => Kind::Paragraph,
        "h1" => Kind::Heading(1),
        "h2" => Kind::Heading(2),
        "h3" => Kind::Heading(3),
        "h4" => Kind::Heading(4),
        "h5" => Kind::Heading(5),
        "h6" => Kind::Heading(6),
        "blockquote" => Kind::Quote,
        "ul" | "ol" => Kind::List,
        "li" => Kind::Item,
        "pre" => Kind::Pre,
        "table" => Kind::Table,
        "tr" => Kind::Row,
        "td" | "th" => Kind::Cell,
        "a" => Kind::Anchor,
        "b" | "strong" => Kind::Strong,
        "i" | "em" | "cite" | "dfn" | "var" => Kind::Emphasis,
        "code" | "kbd" | "samp" | "tt" => Kind::Code,
        "abbr" | "acronym" | "bdi" | "bdo" | "data" | "del" | "ins" | "mark" | "q" | "rp"
        | "rt" | "rtc" | "ruby" | "s" | "small" | "span" | "strike" | "sub" | "sup" | "time"
        | "u" => Kind::Inline,
        "article" | "aside" | "center" | "dd" | "details" | "div" | "dl" | "dt" | "figcaption"
        | "figure" | "footer" | "header" | "hgroup" | "nav" | "caption" | "summary" | "map" => {
            Kind::Blocks
        }
        _ => return None,
    })
}

pub(super) const MAPPED: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "area",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "map",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "rtc",
    "ruby",
    "s",
    "samp",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "time",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "wbr",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Root,
    Blocks,
    Paragraph,
    Heading(u8),
    Quote,
    List,
    Item,
    Pre,
    Table,
    Row,
    Cell,
    Anchor,
    Strong,
    Emphasis,
    Code,
    Inline,
}

impl Kind {
    pub(super) fn closes_when_block_opens(self) -> bool {
        matches!(
            self,
            Self::Paragraph
                | Self::Heading(_)
                | Self::Anchor
                | Self::Strong
                | Self::Emphasis
                | Self::Code
                | Self::Inline
        )
    }

    pub(super) fn accepts_blocks(self) -> bool {
        matches!(
            self,
            Self::Root | Self::Blocks | Self::Quote | Self::Item | Self::Cell | Self::List
        )
    }

    pub(super) fn is_inline(self) -> bool {
        matches!(
            self,
            Self::Anchor | Self::Strong | Self::Emphasis | Self::Code | Self::Inline
        )
    }

    pub(super) fn is_block(self) -> bool {
        matches!(
            self,
            Self::Blocks
                | Self::Paragraph
                | Self::Heading(_)
                | Self::Quote
                | Self::List
                | Self::Item
                | Self::Pre
                | Self::Table
        )
    }
}

pub fn is_mapped(name: &str) -> bool {
    open_as(name).is_some()
}

pub fn mapped_tags() -> &'static [&'static str] {
    MAPPED
}

pub(super) struct Frame {
    pub(super) kind: Kind,
    pub(super) name: String,
    pub(super) forced: Option<Dir>,
    pub(super) buf: String,
    pub(super) spans: Vec<Span>,
    pub(super) blocks: Vec<Block>,
    pub(super) items: Vec<Vec<Block>>,
    pub(super) rows: Vec<Row>,
    pub(super) cells: Vec<Cell>,
    pub(super) ordered: bool,
    pub(super) layout: bool,
    pub(super) in_head: bool,
    pub(super) link: Option<SafeUrl>,
    pub(super) header: bool,
    pub(super) lang: Option<String>,
    pub(super) raw: bool,
}

pub(super) struct Row {
    pub(super) cells: Vec<Cell>,
    pub(super) head: bool,
}

pub(super) struct Cell {
    pub(super) blocks: Vec<Block>,
    pub(super) header: bool,
}

impl Frame {
    pub(super) fn root() -> Self {
        Self::new(Kind::Root, "", None)
    }

    pub(super) fn new(kind: Kind, name: &str, forced: Option<Dir>) -> Self {
        Self {
            kind,
            name: name.to_owned(),
            forced,
            buf: String::new(),
            spans: Vec::new(),
            blocks: Vec::new(),
            items: Vec::new(),
            rows: Vec::new(),
            cells: Vec::new(),
            ordered: false,
            layout: false,
            in_head: false,
            link: None,
            header: false,
            lang: None,
            raw: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAPPED, open_as};

    #[test]
    fn every_listed_tag_has_a_role() {
        for name in MAPPED {
            assert!(open_as(name).is_some(), "{name} is listed but unmapped");
        }
    }
}
