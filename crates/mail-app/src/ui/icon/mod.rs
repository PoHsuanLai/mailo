//! Lucide glyphs for the shell.
//!
//! The geometry is Lucide (<https://lucide.dev>), ISC licence, notice in
//! `crates/mail-app/assets/icons/LICENSE-lucide.txt`.
//!
//! The style is a 24 grid, 2px stroke, round caps and joins, one colour, no fills,
//! with the stroke drawn by CSS `.ic`, not by attributes.
//!
//! The icons are data and not markup strings, so nothing here needs a raw HTML sink.
//! The geometry itself is in [`geometry`], transcribed from the design's `ICON` table.
//!
//! No serde: an icon is never stored, and a derive would make it a persisted schema
//! (`CONVENTIONS.md` section 3).

mod geometry;

use dioxus::prelude::*;
use geometry::*;

/// One Lucide glyph, named for its key in the design's `ICON` table.
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub(super) enum Icon {
    Inbox,
    Star,
    Archive,
    Clock,
    Trash,
    Mail,
    MailOpen,
    Tag,
    Refresh,
    Send,
    Command,
    Columns,
    Group,
    Panel,
    Square,
    Maximize,
    Corner,
    Undo,
    Check,
    X,
    Paperclip,
    Pen,
    Key,
    FilePen,
    OctagonAlert,
    Pin,
    Reply,
    ReplyAll,
    Forward,
    Search,
    Settings,
    PanelLeft,
    Plus,
    Printer,
    FolderInput,
}

impl Icon {
    /// Every glyph, in the order of the keys of `ICON`.
    ///
    /// The icon tests lock this list to `icons.js`. The const below reads it so the binary
    /// constructs every variant, not only the six the reader draws.
    pub(super) const ALL: &[Icon] = &[
        Icon::Inbox,
        Icon::Star,
        Icon::Archive,
        Icon::Clock,
        Icon::Trash,
        Icon::Mail,
        Icon::MailOpen,
        Icon::Tag,
        Icon::Refresh,
        Icon::Send,
        Icon::Command,
        Icon::Columns,
        Icon::Group,
        Icon::Panel,
        Icon::Square,
        Icon::Maximize,
        Icon::Corner,
        Icon::Undo,
        Icon::Check,
        Icon::X,
        Icon::Paperclip,
        Icon::Pen,
        Icon::Key,
        Icon::FilePen,
        Icon::OctagonAlert,
        Icon::Pin,
        Icon::Reply,
        Icon::ReplyAll,
        Icon::Forward,
        Icon::Search,
        Icon::Settings,
        Icon::PanelLeft,
        Icon::Plus,
        Icon::Printer,
        Icon::FolderInput,
    ];

    /// The children of this glyph, in the design's order.
    pub(super) fn shapes(self) -> &'static [Shape] {
        match self {
            Icon::Inbox => INBOX,
            Icon::Star => STAR,
            Icon::Archive => ARCHIVE,
            Icon::Clock => CLOCK,
            Icon::Trash => TRASH,
            Icon::Mail => MAIL,
            Icon::MailOpen => MAIL_OPEN,
            Icon::Tag => TAG,
            Icon::Refresh => REFRESH,
            Icon::Send => SEND,
            Icon::Command => COMMAND,
            Icon::Columns => COLUMNS,
            Icon::Group => GROUP,
            Icon::Panel => PANEL,
            Icon::Square => SQUARE,
            Icon::Maximize => MAXIMIZE,
            Icon::Corner => CORNER,
            Icon::Undo => UNDO,
            Icon::Check => CHECK,
            Icon::X => X_MARK,
            Icon::Paperclip => PAPERCLIP,
            Icon::Pen => PEN,
            Icon::Key => KEY,
            Icon::FilePen => FILE_PEN,
            Icon::OctagonAlert => OCTAGON_ALERT,
            Icon::Pin => PIN,
            Icon::Reply => REPLY,
            Icon::ReplyAll => REPLY_ALL,
            Icon::Forward => FORWARD,
            Icon::Search => SEARCH,
            Icon::Settings => SETTINGS,
            Icon::PanelLeft => PANEL_LEFT,
            Icon::Plus => PLUS,
            Icon::Printer => PRINTER,
            Icon::FolderInput => FOLDER_INPUT,
        }
    }
}

/// A variant only matched in [`Icon::shapes`] is still dead until something constructs it.
/// [`Icon::ALL`] constructs every glyph; reading it here keeps the sidebar's icons in the
/// binary before that pane calls them.
const _: usize = Icon::ALL.len();

/// One SVG child. Numbers are the design's decimal text, so `.6` stays `.6`.
///
/// A rect attribute the design omits is `"0"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Shape {
    /// An outline, the path `d`.
    Path(&'static str),
    /// A circle on the 24 grid.
    Circle {
        cx: &'static str,
        cy: &'static str,
        r: &'static str,
    },
    /// A rectangle on the 24 grid.
    Rect {
        x: &'static str,
        y: &'static str,
        width: &'static str,
        height: &'static str,
        rx: &'static str,
    },
}

fn child(shape: &Shape) -> Element {
    match shape {
        Shape::Path(d) => rsx! { path { d: "{d}" } },
        Shape::Circle { cx, cy, r } => rsx! { circle { cx: "{cx}", cy: "{cy}", r: "{r}" } },
        Shape::Rect {
            x,
            y,
            width,
            height,
            rx,
        } => rsx! {
            rect {
                x: "{x}",
                y: "{y}",
                width: "{width}",
                height: "{height}",
                rx: "{rx}",
            }
        },
    }
}

/// `icon`, drawn as an `svg` of class `ic`, plus `class` when one is given.
///
/// Stroke, caps, joins and the empty fill come from CSS `.ic`.
#[component]
pub(super) fn Glyph(icon: Icon, class: Option<&'static str>) -> Element {
    let class = match class {
        Some(extra) => format!("ic {extra}"),
        None => "ic".to_owned(),
    };
    rsx! {
        svg {
            class: "{class}",
            view_box: "0 0 24 24",
            // SVG elements do not carry the HTML `aria_hidden` attribute.
            "aria-hidden": "true",
            for shape in icon.shapes() {
                {child(shape)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Glyph, GlyphProps, Icon, Shape};
    use dioxus::prelude::*;

    fn markup(icon: Icon) -> String {
        let mut dom = VirtualDom::new_with_props(Glyph, GlyphProps { icon, class: None });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Opening-tag names, in order. A close tag is not an element.
    fn element_names(html: &str) -> Vec<&str> {
        let bytes = html.as_bytes();
        let mut names = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'<' {
                index += 1;
                continue;
            }
            index += 1;
            if index >= bytes.len() || matches!(bytes[index], b'/' | b'!' | b'?') {
                continue;
            }
            let start = index;
            while index < bytes.len()
                && !matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/')
            {
                index += 1;
            }
            names.push(&html[start..index]);
        }
        names
    }

    fn shape_tag(shape: &Shape) -> &'static str {
        match shape {
            Shape::Path(_) => "path",
            Shape::Circle { .. } => "circle",
            Shape::Rect { .. } => "rect",
        }
    }

    #[test]
    fn every_icon_is_an_svg_of_its_shapes() {
        let mut failures = Vec::new();
        for icon in Icon::ALL {
            let page = markup(*icon);
            let shapes = icon.shapes();
            let names = element_names(&page);
            let expect: Vec<&str> = std::iter::once("svg")
                .chain(shapes.iter().map(shape_tag))
                .collect();
            if shapes.is_empty()
                || names != expect
                || !page.contains("<svg")
                || !page.contains("class=\"ic\"")
                || !page.contains("viewBox=\"0 0 24 24\"")
                || !page.contains("aria-hidden=\"true\"")
            {
                failures.push(format!(
                    "{icon:?}: {} shapes, names {names:?}, page {page}",
                    shapes.len()
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn the_set_matches_icons_js() {
        // The first 23 are the keys of `ICON` in mailo-design/icons.js (inbox through key).
        // The rest are the glyphs the frame's places, rows and foot draw, fetched from Lucide.
        // Plus is the Spaces mockup's `ICON.plus`: the foot's "New Space". Printer is Lucide's
        // `printer`, the reader head's Print; FolderInput is Lucide's `folder-input`, Move to….
        const KEYS_IN_ICONS_JS: usize = 23;
        const FRAME_GLYPHS: usize = 12;
        assert_eq!(
            Icon::ALL.len(),
            KEYS_IN_ICONS_JS + FRAME_GLYPHS,
            "{:?}",
            Icon::ALL
        );
        let mut duplicates = Vec::new();
        for (index, icon) in Icon::ALL.iter().enumerate() {
            if Icon::ALL[index + 1..].contains(icon) {
                duplicates.push(format!("{icon:?}"));
            }
        }
        assert!(
            duplicates.is_empty(),
            "duplicates: {}",
            duplicates.join(", ")
        );
    }

    /// First path `d` of star, check and undo, copied from `icons.js`.
    const FIRST_PATH: &[(Icon, &str)] = &[
        (
            Icon::Star,
            "M12 2.5l2.9 5.88 6.49.94-4.7 4.58 1.11 6.46L12 17.31l-5.8 3.05 1.1-6.46-4.69-4.58 6.48-.94z",
        ),
        (Icon::Check, "M20 6 9 17l-5-5"),
        (Icon::Undo, "M9 14 4 9l5-5"),
    ];

    #[test]
    fn star_check_and_undo_keep_their_first_path() {
        let mut failures = Vec::new();
        for (icon, expect) in FIRST_PATH {
            let got = match icon.shapes().first() {
                Some(Shape::Path(d)) => Some(*d),
                _ => None,
            };
            if got != Some(*expect) {
                failures.push(format!("{icon:?}: first path is {got:?}"));
            }
            let page = markup(*icon);
            let needle = format!("d=\"{expect}\"");
            if !page.contains(&needle) {
                failures.push(format!("{icon:?} rendered without {needle}: {page}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
