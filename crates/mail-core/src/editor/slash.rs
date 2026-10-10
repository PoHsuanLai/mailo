//! The `/` menu: turn a line into a paragraph kind, or insert an object.
//!
//! Filtering is prefix on the name, then prefix on a keyword, then [`crate::search::match_list`].
//! `search::fuzzy` is private; the matcher is the one the command menu re-exports. An empty
//! query lists the whole menu, in catalog order.

use crate::editor::doc::{Check, Level, ParaKind};
use crate::search::match_list;

/// What choosing the item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Turn the current paragraph into this kind. This is the Turn into menu.
    Turn(ParaKind),
    /// Insert a divider.
    Divider,
    /// Insert an image.
    Image,
    /// Insert a table.
    Table,
    /// Attach a file.
    Attachment,
    /// Insert the signature separator.
    Signature,
    /// Insert the thanks snippet.
    Snippet,
    /// Insert today's date. The wording is the caller's: this crate does not read a clock.
    Date,
}

/// One row of the slash menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    /// The row's name.
    pub name: &'static str,
    /// The line of help under the name.
    pub help: &'static str,
    /// Extra words the filter treats as names.
    pub keywords: &'static [&'static str],
    /// Icon name for the tile.
    pub icon: &'static str,
    /// The Markdown that does the same as you type, shown at the row's right. Often empty.
    pub markdown: &'static str,
    /// What the row does.
    pub action: Action,
}

const ITEMS: &[Item] = &[
    Item {
        name: "Text",
        help: "Plain paragraph.",
        keywords: &["paragraph", "plain"],
        icon: "type",
        markdown: "",
        action: Action::Turn(ParaKind::Paragraph),
    },
    Item {
        name: "Heading 1",
        help: "A section heading.",
        keywords: &["h1", "title", "heading"],
        icon: "heading-1",
        markdown: "#",
        action: Action::Turn(ParaKind::Heading(Level::One)),
    },
    Item {
        name: "Heading 2",
        help: "A smaller heading.",
        keywords: &["h2", "subheading", "heading"],
        icon: "heading-2",
        markdown: "##",
        action: Action::Turn(ParaKind::Heading(Level::Two)),
    },
    Item {
        name: "Heading 3",
        help: "The smallest heading.",
        keywords: &["h3", "heading"],
        icon: "heading-3",
        markdown: "###",
        action: Action::Turn(ParaKind::Heading(Level::Three)),
    },
    Item {
        name: "Bullet list",
        help: "A simple list.",
        keywords: &["ul", "list", "bullet"],
        icon: "list",
        markdown: "-",
        action: Action::Turn(ParaKind::Bullet),
    },
    Item {
        name: "Numbered list",
        help: "A list in order.",
        keywords: &["ol", "list", "number"],
        icon: "list-ordered",
        markdown: "1.",
        action: Action::Turn(ParaKind::Numbered),
    },
    Item {
        name: "To-do",
        help: "Boxes, sent as ☐ and ☑.",
        keywords: &["todo", "task", "checkbox"],
        icon: "check-square",
        markdown: "[]",
        action: Action::Turn(ParaKind::Todo(Check::Open)),
    },
    Item {
        name: "Quote",
        help: "Set a passage apart.",
        keywords: &["blockquote"],
        icon: "quote",
        markdown: ">",
        action: Action::Turn(ParaKind::Quote),
    },
    Item {
        name: "Code",
        help: "Monospace, sent as-is.",
        keywords: &["pre", "monospace"],
        icon: "code",
        markdown: "```",
        action: Action::Turn(ParaKind::Code),
    },
    Item {
        name: "Divider",
        help: "A quiet line.",
        keywords: &["hr", "line", "rule"],
        icon: "minus",
        markdown: "---",
        action: Action::Divider,
    },
    Item {
        name: "Image",
        help: "Embedded, never fetched.",
        keywords: &["picture", "photo", "img"],
        icon: "image",
        markdown: "",
        action: Action::Image,
    },
    Item {
        name: "Table",
        help: "Rows and columns, kept simple.",
        keywords: &["grid", "columns"],
        icon: "table",
        markdown: "",
        action: Action::Table,
    },
    Item {
        name: "Attachment",
        help: "Adds a file to the message.",
        keywords: &["file", "attach"],
        icon: "paperclip",
        markdown: "",
        action: Action::Attachment,
    },
    Item {
        name: "Signature",
        help: "This identity's signature.",
        keywords: &["sign"],
        icon: "signature",
        markdown: "",
        action: Action::Signature,
    },
    Item {
        name: "Snippet",
        help: "Thanks for the quick reply —",
        keywords: &["thanks", "snippet"],
        icon: "pen",
        markdown: "",
        action: Action::Snippet,
    },
    Item {
        name: "Date",
        help: "Today, written out.",
        keywords: &["today", "time"],
        icon: "calendar",
        markdown: "",
        action: Action::Date,
    },
];

/// The whole menu, in catalog order.
pub fn catalog() -> &'static [Item] {
    ITEMS
}

/// The same rows restricted to paragraph kinds. This is Turn into.
pub fn turn_into() -> Vec<&'static Item> {
    ITEMS
        .iter()
        .filter(|item| matches!(item.action, Action::Turn(_)))
        .collect()
}

/// Items matching `query`.
///
/// An empty query is the whole catalog. Otherwise names that start with the query come
/// first, then keywords that start with it, then a fuzzy match on the name and keywords.
pub fn filter(query: &str) -> Vec<&'static Item> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return ITEMS.iter().collect();
    }
    let mut out = Vec::new();
    let mut seen = vec![false; ITEMS.len()];
    for (index, item) in ITEMS.iter().enumerate() {
        if item.name.to_lowercase().starts_with(&query) {
            out.push(item);
            seen[index] = true;
        }
    }
    for (index, item) in ITEMS.iter().enumerate() {
        if seen[index] {
            continue;
        }
        if item
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().starts_with(&query))
        {
            out.push(item);
            seen[index] = true;
        }
    }
    let labels: Vec<String> = ITEMS
        .iter()
        .map(|item| {
            let mut label = item.name.to_owned();
            for keyword in item.keywords {
                label.push(' ');
                label.push_str(keyword);
            }
            label
        })
        .collect();
    let haystacks: Vec<&str> = labels.iter().map(String::as_str).collect();
    for hit in match_list(&query, &haystacks) {
        if !seen[hit.index] {
            out.push(&ITEMS[hit.index]);
            seen[hit.index] = true;
        }
    }
    out
}

/// The thanks snippet. The date is not here: writing "today" needs a clock the caller has.
pub const SNIPPET: &str = "Thanks for the quick reply — ";
