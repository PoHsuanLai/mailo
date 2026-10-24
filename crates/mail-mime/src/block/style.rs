//! The few styles a composed message carries, and the only CSS the block walk reads.
//!
//! A paragraph may be aligned and indented; a run of text may be a size and a colour. Each is
//! one of a fixed set, written as one fixed declaration, and read back only when the value is
//! exactly one of those: `font-size` from [`TextSize`], `color` from [`TextColor`]'s palette,
//! `text-align` from [`Align`] and `margin-left` in [`ParaStyle::INDENT_PX`] steps. Any other
//! declaration, and any other value of these four, is not read and does not survive
//! [`crate::sanitize`] under [`crate::Styles::Dropped`]: a sender's CSS never reaches the blocks.

/// How a paragraph's lines sit across the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    /// Ragged right: no declaration is written.
    #[default]
    Left,
    /// Centred.
    Center,
    /// Ragged left.
    Right,
}

impl Align {
    /// Every alignment, in menu order.
    pub const ALL: [Align; 3] = [Align::Left, Align::Center, Align::Right];

    /// The `text-align` value.
    pub fn css(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }

    /// The alignment a `text-align` value names, when it is one of the three.
    pub fn from_css(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|align| align.css() == value)
    }
}

/// A paragraph's alignment and its indent, in steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ParaStyle {
    /// Its alignment.
    pub align: Align,
    /// Steps of [`ParaStyle::INDENT_PX`] from the left, at most [`ParaStyle::MAX_INDENT`].
    pub indent: u8,
}

impl ParaStyle {
    /// One indent step, as `margin-left`.
    pub const INDENT_PX: u32 = 40;
    /// The deepest indent, list nesting and quote level a message keeps.
    pub const MAX_INDENT: u8 = 6;

    /// The `style` attribute's value, `None` for a plain paragraph.
    pub fn css(self) -> Option<String> {
        let mut out = Vec::new();
        if self.indent > 0 {
            out.push(format!(
                "margin-left:{}px",
                u32::from(self.indent.min(Self::MAX_INDENT)) * Self::INDENT_PX
            ));
        }
        if self.align != Align::Left {
            out.push(format!("text-align:{}", self.align.css()));
        }
        (!out.is_empty()).then(|| out.join(";"))
    }
}

/// A run's size, as the four steps Apple Mail and Gmail offer: these three and normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextSize {
    /// Smaller than the body.
    Small,
    /// Larger than the body.
    Large,
    /// Much larger.
    Huge,
}

impl TextSize {
    /// Every size but normal, smallest first.
    pub const ALL: [TextSize; 3] = [TextSize::Small, TextSize::Large, TextSize::Huge];

    /// The `font-size` value: a keyword, so the reader's own body size is what it scales.
    pub fn css(self) -> &'static str {
        match self {
            TextSize::Small => "small",
            TextSize::Large => "large",
            TextSize::Huge => "x-large",
        }
    }

    /// The size a `font-size` value names, when it is one of the three.
    pub fn from_css(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.css() == value)
    }
}

/// A run's colour: a small palette that reads on a light page and a dark one. No white and no
/// black, which would vanish on one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextColor {
    /// Grey.
    Gray,
    /// Red.
    Red,
    /// Orange.
    Orange,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Purple.
    Purple,
}

impl TextColor {
    /// The palette, in swatch order.
    pub const ALL: [TextColor; 6] = [
        TextColor::Gray,
        TextColor::Red,
        TextColor::Orange,
        TextColor::Green,
        TextColor::Blue,
        TextColor::Purple,
    ];

    /// The `color` value, lower-case hex.
    pub fn css(self) -> &'static str {
        match self {
            TextColor::Gray => "#80868b",
            TextColor::Red => "#d93025",
            TextColor::Orange => "#e8710a",
            TextColor::Green => "#188038",
            TextColor::Blue => "#1a73e8",
            TextColor::Purple => "#9334e6",
        }
    }

    /// The colour a `color` value names, when it is one of the palette's, in either case.
    pub fn from_css(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|color| color.css().eq_ignore_ascii_case(value))
    }
}

/// A run's size and colour. Both `None` is plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RunStyle {
    /// Its size, `None` for the body's.
    pub size: Option<TextSize>,
    /// Its colour, `None` for the text's own.
    pub color: Option<TextColor>,
}

impl RunStyle {
    /// Neither a size nor a colour.
    pub fn is_plain(self) -> bool {
        self.size.is_none() && self.color.is_none()
    }

    /// The `style` attribute's value, `None` for plain text.
    pub fn css(self) -> Option<String> {
        let mut out = Vec::new();
        if let Some(size) = self.size {
            out.push(format!("font-size:{}", size.css()));
        }
        if let Some(color) = self.color {
            out.push(format!("color:{}", color.css()));
        }
        (!out.is_empty()).then(|| out.join(";"))
    }
}

/// What a `style` attribute says that this module reads: a paragraph's alignment and indent,
/// and a run's size and colour. Everything else in it is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Read {
    pub(crate) align: Option<Align>,
    pub(crate) indent: u8,
    pub(crate) run: RunStyle,
}

/// The declarations of `style` this module reads, each from its fixed set. A later declaration
/// of the same property wins, as in CSS; one whose value is not in the set is left out.
pub(crate) fn read(style: &str) -> Read {
    let mut out = Read::default();
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim().to_ascii_lowercase();
        match property.trim().to_ascii_lowercase().as_str() {
            "text-align" => {
                if let Some(align) = Align::from_css(&value) {
                    out.align = Some(align);
                }
            }
            "margin-left" => {
                if let Some(steps) = indent_steps(&value) {
                    out.indent = steps;
                }
            }
            "font-size" => {
                if let Some(size) = TextSize::from_css(&value) {
                    out.run.size = Some(size);
                }
            }
            "color" => {
                if let Some(color) = TextColor::from_css(&value) {
                    out.run.color = Some(color);
                }
            }
            _ => {}
        }
    }
    out
}

/// `margin-left` as indent steps: a whole multiple of [`ParaStyle::INDENT_PX`] in `px`, from
/// one step to [`ParaStyle::MAX_INDENT`].
fn indent_steps(value: &str) -> Option<u8> {
    // A value that is not a whole number of pixels is not an indent this module wrote.
    let px: u32 = value.strip_suffix("px")?.parse().ok()?;
    if px == 0 || !px.is_multiple_of(ParaStyle::INDENT_PX) {
        return None;
    }
    u8::try_from(px / ParaStyle::INDENT_PX)
        // More steps than a u8 holds is past the deepest indent anyway.
        .ok()
        .filter(|steps| *steps <= ParaStyle::MAX_INDENT)
}

/// `style` with only what [`read`] reads, written back in the fixed form; `None` when nothing
/// is left. What the sanitizer keeps of a `style` attribute when the sender's CSS is dropped.
pub(crate) fn allowed(style: &str) -> Option<String> {
    let read = read(style);
    let para = ParaStyle {
        // No alignment read is left, which is written as nothing.
        align: read.align.unwrap_or_default(),
        indent: read.indent,
    };
    let parts: Vec<String> = [para.css(), read.run.css()].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_fixed_values_are_read() {
        // (style, what survives)
        let cases: &[(&str, Option<&str>)] = &[
            ("text-align:center", Some("text-align:center")),
            ("TEXT-ALIGN: Right ;", Some("text-align:right")),
            ("text-align:left", None),
            ("text-align:justify", None),
            ("margin-left:40px", Some("margin-left:40px")),
            ("margin-left:240px", Some("margin-left:240px")),
            ("margin-left:280px", None),
            ("margin-left:41px", None),
            ("margin-left:2em", None),
            ("margin-left:-40px", None),
            ("font-size:large", Some("font-size:large")),
            ("font-size:x-large", Some("font-size:x-large")),
            ("font-size:13px", None),
            ("font-size:xx-large", None),
            ("color:#D93025", Some("color:#d93025")),
            ("color:#d93026", None),
            ("color:red", None),
            ("color:#fff", None),
            ("background:url(https://t.example/p.gif)", None),
            ("position:fixed;top:0", None),
            (
                "color:#1a73e8;font-size:small;text-align:center;margin-left:80px;display:none",
                Some("margin-left:80px;text-align:center;font-size:small;color:#1a73e8"),
            ),
            ("color:#1a73e8;color:#bad", Some("color:#1a73e8")),
            ("", None),
            ("garbage", None),
        ];
        for (style, kept) in cases {
            assert_eq!(allowed(style).as_deref(), *kept, "{style}");
        }
    }

    #[test]
    fn every_value_written_reads_back() {
        for align in Align::ALL {
            for indent in 0..=ParaStyle::MAX_INDENT {
                let para = ParaStyle { align, indent };
                let read = read(&para.css().unwrap_or_default());
                assert_eq!(read.align.unwrap_or_default(), align);
                assert_eq!(read.indent, indent);
            }
        }
        for size in TextSize::ALL.into_iter().map(Some).chain([None]) {
            for color in TextColor::ALL.into_iter().map(Some).chain([None]) {
                let run = RunStyle { size, color };
                assert_eq!(read(&run.css().unwrap_or_default()).run, run);
            }
        }
    }
}
