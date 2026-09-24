//! The gradients a new Space is tinted from: quire's eight presets, the ones its Space editor
//! offers.
//!
//! The six Part A accent hues mailo once kept among them (Postmark, Graphite, Pine, Indigo,
//! Oxblood, Vermilion) are retired: the editor is quire's, and it offers quire's eight. A Space
//! made from one of them keeps its dots; only the preset is gone.

use ds::Dot;

/// quire's presets' dots, in their order (design/21-SPACES.md section 4).
pub const PRESETS: [&[Dot]; 8] = [
    ds::PRESETS[0].dots,
    ds::PRESETS[1].dots,
    ds::PRESETS[2].dots,
    ds::PRESETS[3].dots,
    ds::PRESETS[4].dots,
    ds::PRESETS[5].dots,
    ds::PRESETS[6].dots,
    ds::PRESETS[7].dots,
];

/// What the editor calls each preset, in [`PRESETS`] order: quire's names.
pub const PRESET_NAMES: [&str; 8] = [
    ds::PRESETS[0].name,
    ds::PRESETS[1].name,
    ds::PRESETS[2].name,
    ds::PRESETS[3].name,
    ds::PRESETS[4].name,
    ds::PRESETS[5].name,
    ds::PRESETS[6].name,
    ds::PRESETS[7].name,
];

#[cfg(test)]
mod tests;
