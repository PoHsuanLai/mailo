//! The gradients a new Space is tinted from, and the six retired accent hues.

use ds::Dot;

/// The eight gradients from the mockup, then the six Part A accent hues as single dots.
///
/// The accent dots sit at chroma 0.7, except graphite, whose own colour is almost
/// neutral and is stored at 0.08. Each hue is the OKLCH hue of that Part A swatch.
pub const PRESETS: &[&[Dot]] = &[
    &[
        Dot {
            hue: 268.0,
            chroma: 0.72,
        },
        Dot {
            hue: 318.0,
            chroma: 0.55,
        },
    ],
    &[
        Dot {
            hue: 152.0,
            chroma: 0.62,
        },
        Dot {
            hue: 62.0,
            chroma: 0.55,
        },
        Dot {
            hue: 28.0,
            chroma: 0.5,
        },
    ],
    &[Dot {
        hue: 220.0,
        chroma: 0.7,
    }],
    &[
        Dot {
            hue: 20.0,
            chroma: 0.66,
        },
        Dot {
            hue: 55.0,
            chroma: 0.6,
        },
    ],
    &[
        Dot {
            hue: 190.0,
            chroma: 0.6,
        },
        Dot {
            hue: 240.0,
            chroma: 0.55,
        },
    ],
    &[
        Dot {
            hue: 340.0,
            chroma: 0.6,
        },
        Dot {
            hue: 290.0,
            chroma: 0.5,
        },
    ],
    &[Dot {
        hue: 95.0,
        chroma: 0.5,
    }],
    &[Dot {
        hue: 250.0,
        chroma: 0.06,
    }],
    &[Dot {
        hue: 257.437_8,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 137.85431,
        chroma: 0.08,
    }],
    &[Dot {
        hue: 164.06635,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 276.64212,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 22.80671,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 32.172_4,
        chroma: 0.7,
    }],
];

/// What the editor calls each preset, in [`PRESETS`] order.
///
/// The last six are the accent hues the window used to offer on their own, kept by name so
/// someone who chose Pine still finds Pine.
pub const PRESET_NAMES: &[&str] = &[
    "Dusk",
    "Orchard",
    "Harbour",
    "Ember",
    "Lagoon",
    "Heather",
    "Moss",
    "Stone",
    "Postmark",
    "Graphite",
    "Pine",
    "Indigo",
    "Oxblood",
    "Vermilion",
];

#[cfg(test)]
mod tests;
