//! The search panel's rows as quire draws a result list: each section under its header with a
//! rule between sections, a row's title with the characters the query matched in semibold, a
//! second line under it (a mail's sender and snippet, a person's address), and an avatar or a
//! glyph before it. Pure: mailo's rows in, quire's menu items out.

use super::super::menu::{MenuItem, Right, Tile};
use super::super::sidebar::hex_colour;
use ds::components::content::avatar::{AvatarFace, AvatarShape, AvatarSize, AvatarTone};
use ds::prelude::{Marks, MenuImage};

/// A quire menu item whose pick hands back the row's key.
pub(super) type Line = ds::prelude::MenuItem<String>;

/// The avatar a result row leads with: a menu's rich tile, smaller than the reader's.
const FACE: AvatarSize = AvatarSize::Size28;

/// `rows` in the panel's order as quire's items: a header where the group changes, a rule
/// before every header but the first.
pub(super) fn suggestions(rows: &[MenuItem]) -> Vec<Line> {
    let mut out = Vec::new();
    let mut last: Option<&str> = None;
    for row in rows {
        if let Some(group) = row.group.as_deref()
            && last != Some(group)
        {
            if last.is_some() {
                out.push(Line::Separator);
            }
            out.push(Line::Header(group.to_owned()));
            last = Some(group);
        }
        out.push(suggestion(row));
    }
    out
}

/// One row: its name with the matched characters marked, its second line, its picture, and a
/// command's shortcut as the faint word at the end.
pub(super) fn suggestion(row: &MenuItem) -> Line {
    let mut line = Line::new(row.key.clone(), row.name.clone()).with_marks(marks(&row.marks));
    if let Some(subtitle) = subtitle(row) {
        line = line.with_subtitle(subtitle);
    }
    if let Some(image) = image(&row.tile) {
        line = line.with_image(image);
    }
    match &row.right {
        Right::Shortcut(text) | Right::Hint(text) => line.with_hint(text.clone()),
        Right::Check(_) | Right::Remove(_) | Right::None => line,
    }
}

/// The marked characters, as ranges quire merges where they touch.
fn marks(indices: &[u32]) -> Marks {
    Marks::over(indices.iter().map(|&at| {
        let at = at as usize;
        at..at + 1
    }))
}

/// The second line: the row's detail runs as one line (a mail's "sender · snippet", a person's
/// "address · threads"), else its help (a Space's "Space").
fn subtitle(row: &MenuItem) -> Option<String> {
    if row.detail.is_empty() {
        return row.help.clone();
    }
    Some(row.detail.iter().map(|run| run.text.as_str()).collect())
}

fn image(tile: &Tile) -> Option<MenuImage> {
    match tile {
        Tile::Icon(icon) => Some(MenuImage::Icon(*icon)),
        Tile::Avatar { letter, color } => Some(MenuImage::Avatar(AvatarFace {
            initial: *letter,
            size: FACE,
            tone: AvatarTone::Account(hex_colour(color)),
            shape: AvatarShape::Round,
        })),
        Tile::Glyph(_) | Tile::Text(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::menu::{Run, Tone};
    use super::*;
    use ds::prelude::Icon;

    fn row(key: &str, name: &str, group: &str) -> MenuItem {
        MenuItem {
            key: key.to_owned(),
            tile: Tile::Icon(Icon::Command),
            name: name.to_owned(),
            help: None,
            right: Right::None,
            group: Some(group.to_owned()),
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        }
    }

    #[test]
    fn sections_get_a_header_each_and_a_rule_between_them() {
        let rows = vec![
            row("mail:1", "Lunch", "Top Hit"),
            row("mail:2", "Lunch again", "Mail"),
            row("mail:3", "Lunch, third", "Mail"),
            row("action:Compose", "Compose", "Commands"),
        ];
        let shape: Vec<String> = suggestions(&rows)
            .iter()
            .map(|line| match line {
                Line::Header(title) => format!("# {title}"),
                Line::Separator => "---".to_owned(),
                Line::Item { value, .. } => value.clone(),
                _ => "?".to_owned(),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "# Top Hit",
                "mail:1",
                "---",
                "# Mail",
                "mail:2",
                "mail:3",
                "---",
                "# Commands",
                "action:Compose"
            ]
        );
    }

    /// What each kind of row draws as: a mail row is its subject marked over its sender, with
    /// an avatar; a command keeps its glyph and its shortcut; a Space says so under its name.
    #[test]
    fn each_row_becomes_its_menu_line() {
        let mail = MenuItem {
            tile: Tile::Avatar {
                letter: 'D',
                color: "#3366aa".to_owned(),
            },
            marks: vec![0, 1, 2],
            detail: vec![
                Run {
                    text: "Dana".to_owned(),
                    marks: Vec::new(),
                    tone: Tone::Strong,
                },
                Run {
                    text: " · see you at noon".to_owned(),
                    marks: Vec::new(),
                    tone: Tone::Plain,
                },
            ],
            ..row("mail:1", "Lunch", "Mail")
        };
        let compose = MenuItem {
            tile: Tile::Icon(Icon::Pen),
            right: Right::Shortcut("\u{2318}N".to_owned()),
            ..row("action:Compose", "Compose", "Commands")
        };
        let space = MenuItem {
            help: Some("Space".to_owned()),
            ..row("space:1", "Work", "Places and People")
        };
        let cases = [
            (
                "a mail",
                mail,
                Line::new("mail:1".to_owned(), "Lunch")
                    .with_marks(Marks::of_query("Lunch", "lun"))
                    .with_subtitle("Dana · see you at noon")
                    .with_image(MenuImage::Avatar(AvatarFace {
                        initial: 'D',
                        size: FACE,
                        tone: AvatarTone::Account(hex_colour("#3366aa")),
                        shape: AvatarShape::Round,
                    })),
            ),
            (
                "a command",
                compose,
                Line::new("action:Compose".to_owned(), "Compose")
                    .with_image(MenuImage::Icon(Icon::Pen))
                    .with_hint("\u{2318}N"),
            ),
            (
                "a Space",
                space,
                Line::new("space:1".to_owned(), "Work")
                    .with_subtitle("Space")
                    .with_image(MenuImage::Icon(Icon::Command)),
            ),
        ];
        for (name, item, want) in cases {
            assert_eq!(suggestion(&item), want, "{name}");
        }
    }
}
