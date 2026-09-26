//! What mailo's own CSS and markup still may not do by quire's lint, and why. Each entry is
//! one selector, compared whole; a reason names why the rule is right for mailo as it stands,
//! and a gap in quire is reported to quire (coherence rule 3), never patched here.

use ds::lint::{Exception, Rule};

/// A tint or nudge on quire's glyph inside mailo's own chrome.
const GLYPH_TINT: &str = "tints or nudges quire's `Glyph` inside mailo's own chrome; `Glyph` takes no colour of its own, and the rule never reaches into a quire component";

pub(super) const STYLE: &[Exception] = &[
    Exception {
        rule: Rule::DsInternals,
        selector: ".consent .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".attachments .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".inv-row dt .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".inv-answered .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::RawFontSize,
        selector: ".c-body .m-code",
        reason: "code in a message is sized against the text it sits in, which no absolute token can follow",
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".c-warn .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".rules-action .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::RawFontSize,
        selector: ".mono",
        reason: "a mono run is sized against the text it sits in, which no absolute token can follow",
    },
];

pub(in crate::ui) const MARKUP: &[Exception] = &[Exception {
    rule: Rule::HexColour,
    selector: "span.av",
    reason: "an account's avatar wears the colour the account was given, which is data",
}];
