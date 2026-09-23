//! A Space's colours, derived from its dots.
//!
//! Pure: dots and a theme in, hex and the occasional `rgba(...)` out. No file
//! and no clock. The arithmetic is the approved mockup's, including the steps
//! it takes when a colour would leave sRGB or fail its contrast floor.

use crate::contrast::ratio;
use serde::{Deserialize, Serialize};

mod readout;

pub use readout::readout;

/// The card's own colours: Post, which a Space's hue never moves.
///
/// The same literals as `tokens.css` and `tokens.dark.css`; `ui::style`'s tests hold the two
/// together. Here so the editor's contrast readout can measure against the card without
/// reading a stylesheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    /// A pane: the list and the reader.
    pub surface: &'static str,
    /// Body text on it.
    pub ink: &'static str,
    /// Postmark, the card's accent when a Space does not lend its hue.
    pub accent: &'static str,
    /// Postmark's tint, behind a selected row.
    pub accent_soft: &'static str,
    /// Text on Postmark.
    pub accent_ink: &'static str,
}

/// The light card.
pub const POST_LIGHT: Card = Card {
    surface: "#F8F9F6",
    ink: "#1A1E1A",
    accent: "#23508F",
    accent_soft: "#DCE5F3",
    accent_ink: "#F4F8FF",
};

/// The dark card.
pub const POST_DARK: Card = Card {
    surface: "#1D211B",
    ink: "#E7EBE3",
    accent: "#7FA6E6",
    accent_soft: "#1E2A44",
    accent_ink: "#0B142A",
};

/// The card for a theme.
pub fn card(dark: bool) -> Card {
    if dark { POST_DARK } else { POST_LIGHT }
}

/// One colour the person placed.
///
/// `chroma` is a fraction of the most the frame will show, not an OKLCH chroma
/// on its own. Lightness belongs to the theme; only these two are chosen.
///
/// `f32` has no `Eq`, so dots compare with [`PartialEq`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dot {
    /// Position on the OKLCH wheel, in degrees.
    pub hue: f32,
    /// How much of the hue to use, from none to the frame's maximum.
    pub chroma: f32,
}

impl Default for Dot {
    fn default() -> Self {
        NEUTRAL_DOT
    }
}

/// The neutral dot an empty Space is read as: hue 250, a trace of chroma.
pub(crate) const NEUTRAL_DOT: Dot = Dot {
    hue: 250.0,
    chroma: 0.06,
};

/// The colours a Space paints: the frame behind the window, the text on it,
/// and the accent the card borrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// One frame colour per dot, left to right across the gradient.
    pub stops: Vec<String>,
    /// The swatch drawn on the dot itself, at the pick's fixed lightness.
    pub picked: Vec<String>,
    /// Sidebar text.
    pub ink: String,
    /// Secondary sidebar text.
    pub soft: String,
    /// The quietest sidebar text: counts, meta.
    pub faint: String,
    /// A row under the pointer. Dark is the literal `rgba(255,255,255,.06)`.
    pub hover: String,
    /// A selected pill. Dark is `rgba(255,255,255,.10)`; light is white at .72.
    pub pill: String,
    /// The accent inside the card, the Space's hue at Postmark's weight.
    pub accent: String,
    /// The accent's tint, behind a selected row.
    pub accent_soft: String,
    /// Text drawn on top of [`Self::accent`].
    pub accent_ink: String,
    /// A stop's chroma was lowered so the text on it stays legible.
    pub capped: bool,
}

/// Lightness, the step to the next stop, and the chroma a fully saturated dot reaches.
struct Frame {
    lightness: f64,
    step: f64,
    chroma: f64,
}

/// Calibrated against palettes Arc exposed as `--arc-palette-*`.
///
/// A light peach Space has background `#F4EBE5` (L 0.945, C 0.013), title
/// `#2E1000` (L 0.22, C 0.06) and pick `#EF8C62` (C 0.13). A mint gradient
/// runs `#D2F3E5`→`#D2EBF3` (L 0.93, C 0.03–0.04). Dark is `#001E15` (L 0.21).
///
/// A light frame therefore sits at L 0.936 with chroma 0.052, and a dark one
/// at L 0.215 with chroma 0.042. Fitting a colour into sRGB steps chroma down
/// by [`GAMUT_STEP`] (0.002). The contrast cap steps it by [`CAP_STEP`] (0.003).
/// The accent's lightness moves by [`ACCENT_STEP`] (0.01). Those three steps
/// are the approved mockup's.
const FRAME_LIGHT: Frame = Frame {
    lightness: 0.936,
    step: -0.012,
    chroma: 0.052,
};

/// The dark frame. See [`FRAME_LIGHT`] for where the numbers come from.
const FRAME_DARK: Frame = Frame {
    lightness: 0.215,
    step: 0.014,
    chroma: 0.042,
};

/// Lightness of the swatch on a dot. The peach pick `#EF8C62` was C 0.13;
/// a fully saturated dot is drawn at L 0.74, C 0.15. See [`FRAME_LIGHT`].
const PICK_L: f64 = 0.74;
/// Lightness of the field's swatches on a dark field, where 0.74 glares. See [`PICK_L`].
const PICK_L_DARK: f64 = 0.66;
/// Chroma of a fully saturated pick. See [`PICK_L`].
const PICK_C: f64 = 0.15;

/// How far `fit` drops chroma when the colour is outside sRGB. See [`FRAME_LIGHT`].
const GAMUT_STEP: f64 = 0.002;
/// How far the contrast cap drops chroma when text would fail. See [`FRAME_LIGHT`].
const CAP_STEP: f64 = 0.003;
/// How far the accent's lightness moves toward passing on the card. See [`FRAME_LIGHT`].
const ACCENT_STEP: f64 = 0.01;

const HOVER_DARK: &str = "rgba(255,255,255,.06)";
const PILL_DARK: &str = "rgba(255,255,255,.10)";
const PILL_LIGHT: &str = "rgba(255,255,255,.72)";

/// `Math.round`: halves go up, including for negatives. Rust's `round` sends
/// halves away from zero, which disagrees at -1.5.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Build a palette from a Space's dots.
///
/// An empty list is the neutral dot (hue 250, chroma 0.06), the same grey as
/// the last of the eight presets. `dark` selects the dark frame.
pub fn derive(dots: &[Dot], dark: bool) -> Palette {
    let neutral = [NEUTRAL_DOT];
    let dots = if dots.is_empty() { &neutral } else { dots };
    let frame = if dark { &FRAME_DARK } else { &FRAME_LIGHT };
    // Empty input was replaced with the neutral dot above.
    let first = dots.first().unwrap_or(&NEUTRAL_DOT);
    let hue0 = f64::from(first.hue);
    let k = f64::from(first.chroma);

    let ink = if dark {
        hex(0.93, 0.018 * k, hue0)
    } else {
        hex(0.22, 0.06 * k, hue0)
    };
    let soft = if dark {
        hex(0.80, 0.03 * k, hue0)
    } else {
        hex(0.40, 0.05 * k, hue0)
    };
    let faint = if dark {
        hex(0.66, 0.035 * k, hue0)
    } else {
        hex(0.52, 0.05 * k, hue0)
    };

    let mut capped = false;
    let stops = dots
        .iter()
        .enumerate()
        .map(|(index, dot)| {
            let lightness = frame.lightness + index as f64 * frame.step;
            let hue = f64::from(dot.hue);
            let mut chroma = f64::from(dot.chroma) * frame.chroma;
            let mut colour = hex(lightness, chroma, hue);
            while chroma > 0.0 && (below(&ink, &colour, 4.5) || below(&faint, &colour, 3.0)) {
                chroma -= CAP_STEP;
                capped = true;
                colour = hex(lightness, chroma, hue);
            }
            colour
        })
        .collect();

    let hover = if dark {
        HOVER_DARK.to_owned()
    } else {
        hex(0.885, 0.026 * k + 0.004, hue0)
    };
    let pill = if dark {
        PILL_DARK.to_owned()
    } else {
        PILL_LIGHT.to_owned()
    };

    let surface = card(dark).surface;
    let mut accent_l = if dark { 0.77 } else { 0.45 };
    let accent_c = 0.045 + 0.035 * k;
    let mut accent = hex(accent_l, accent_c, hue0);
    while below(&accent, surface, 4.5) && accent_l > 0.2 && accent_l < 0.95 {
        accent_l += if dark { ACCENT_STEP } else { -ACCENT_STEP };
        accent = hex(accent_l, accent_c, hue0);
    }
    let accent_soft = if dark {
        hex(0.29, 0.03, hue0)
    } else {
        hex(0.935, 0.018, hue0)
    };
    let accent_ink = if dark {
        hex(0.2, 0.02, hue0)
    } else {
        "#FFFFFF".to_owned()
    };
    let picked = dots
        .iter()
        .map(|dot| hex(PICK_L, f64::from(dot.chroma) * PICK_C, f64::from(dot.hue)))
        .collect();

    Palette {
        stops,
        picked,
        ink,
        soft,
        faint,
        hover,
        pill,
        accent,
        accent_soft,
        accent_ink,
        capped,
    }
}

/// The colour one point of the editor's hue × chroma field is drawn in.
///
/// The pick's own lightness: L 0.74 on a light field and 0.66 on a dark one, at up to the
/// pick's chroma. The editor draws the field from this and computes no colour of its own.
pub fn swatch(dot: Dot, dark: bool) -> String {
    let lightness = if dark { PICK_L_DARK } else { PICK_L };
    hex(
        lightness,
        f64::from(dot.chroma) * PICK_C,
        f64::from(dot.hue),
    )
}

/// The frame's background, as a CSS `linear-gradient`.
///
/// One stop is repeated, so a single-colour Space is still a gradient. Several
/// stops are spread from 0% to 100%.
pub fn gradient(palette: &Palette) -> String {
    let stops = &palette.stops;
    if stops.len() <= 1 {
        let colour = stops.first().map(String::as_str).unwrap_or("");
        return format!("linear-gradient(135deg,{colour},{colour})");
    }
    let last = stops.len() - 1;
    let body = stops
        .iter()
        .enumerate()
        .map(|(index, colour)| {
            let position = js_round(index as f64 / last as f64 * 100.0);
            // A stop's position is a whole number of percent.
            let position = position as i64;
            format!("{colour} {position}%")
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("linear-gradient(135deg,{body})")
}

fn below(fore: &str, back: &str, need: f64) -> bool {
    match ratio(fore, back) {
        Some(measured) => measured < need,
        // Not a hex pair: treat it as failing, the same as a ratio under the floor.
        None => true,
    }
}

fn oklch_to_rgb(lightness: f64, chroma: f64, hue: f64) -> [f64; 3] {
    let radians = hue * std::f64::consts::PI / 180.0;
    let a = chroma * radians.cos();
    let b = chroma * radians.sin();
    let l_ = lightness + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = lightness - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = lightness - 0.0894841775 * a - 1.2914855480 * b;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    [
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    ]
}

fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter()
        .all(|channel| (-0.0005..=1.0005).contains(channel))
}

fn fit(lightness: f64, chroma: f64, hue: f64) -> f64 {
    let mut chroma = chroma;
    while chroma > 0.0 && !in_gamut(oklch_to_rgb(lightness, chroma, hue)) {
        chroma -= GAMUT_STEP;
    }
    chroma.max(0.0)
}

fn encode(channel: f64) -> f64 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.0031308 {
        12.92 * channel
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

fn hex(lightness: f64, chroma: f64, hue: f64) -> String {
    let [red, green, blue] = oklch_to_rgb(lightness, fit(lightness, chroma, hue), hue);
    let mut out = String::with_capacity(7);
    out.push('#');
    for channel in [red, green, blue] {
        push_byte(&mut out, channel_byte(encode(channel)));
    }
    out
}

fn channel_byte(encoded: f64) -> u8 {
    let rounded = js_round(encoded * 255.0);
    if (0.0..=255.0).contains(&rounded) {
        // `js_round` produced a whole number in range.
        rounded as u8
    } else if rounded < 0.0 {
        0
    } else {
        255
    }
}

fn push_byte(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(char::from(HEX[usize::from(byte >> 4)]));
    out.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

#[cfg(test)]
mod tests;
