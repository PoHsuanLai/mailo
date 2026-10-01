//! The gradients a new Space is tinted from: quire's eight presets, the ones its Space editor
//! offers.
//!
//! The six Part A accent hues mailo once kept among them (Postmark, Graphite, Pine, Indigo,
//! Oxblood, Vermilion) are retired: the editor is quire's, and it offers quire's eight. A Space
//! made from one of them keeps its dots; only the preset is gone.

use ds::style::space::palette::Dot;
use ds::style::space::presets::PRESETS as QUIRE;

/// quire's presets' dots, in their order (design/21-SPACES.md section 4).
pub const PRESETS: [&[Dot]; 8] = [
    QUIRE[0].dots,
    QUIRE[1].dots,
    QUIRE[2].dots,
    QUIRE[3].dots,
    QUIRE[4].dots,
    QUIRE[5].dots,
    QUIRE[6].dots,
    QUIRE[7].dots,
];

/// What the editor calls each preset, in [`PRESETS`] order: quire's names.
pub const PRESET_NAMES: [&str; 8] = [
    QUIRE[0].name,
    QUIRE[1].name,
    QUIRE[2].name,
    QUIRE[3].name,
    QUIRE[4].name,
    QUIRE[5].name,
    QUIRE[6].name,
    QUIRE[7].name,
];

#[cfg(test)]
mod tests;
