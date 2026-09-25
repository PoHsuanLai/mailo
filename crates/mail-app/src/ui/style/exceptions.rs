//! What mailo's own CSS and markup still may not do by quire's lint, and why. Each entry is
//! one selector, compared whole; a reason names the gap or the webview-only behaviour it waits
//! on, and a gap is reported to quire (coherence rule 3), never patched here.

use ds::lint::{Exception, Rule};

/// Truncation the webview draws and Blitz does not.
const ELLIPSIS: &str = "text-overflow: ellipsis truncates in the webview Phase A draws on; Phase B moves it to `.ds-truncate` or `text::clip_chars`";
/// Focus the webview matches and Blitz does not.
const FOCUS: &str = "the webview matches :focus-visible and :focus-within; Phase B moves keyboard focus onto `.ds[data-modality=keyboard] :focus`";
/// A tint or nudge on quire's glyph inside mailo's own chrome.
const GLYPH_TINT: &str = "tints or nudges quire's `Glyph` inside mailo's own chrome; `Glyph` takes no colour of its own, and the rule never reaches into a quire component";
/// A button inside a folder row's `<summary>`.
const IN_SUMMARY: &str = "a folder row is a `<summary>`, and a click on a button inside it would also open or close the folder; the button stops that on the click itself, and quire's `Button` and `IconButton` hand the caller a `Press`, not the event (a gap, reported)";
/// The composer's wire.
const WIRE: &str = "the composer's wire is the textarea the editor glue reads the page through, by design, not a field anyone types in";

pub(super) const STYLE: &[Exception] = &[
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".list-bar h2",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".list-bar .status",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".fold-name",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".fold-row:hover .more, .fold-row:focus-within .more, .fold-row .more[*|aria-expanded=\"true\"]",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".fold-acct",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".consent .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".attachments .name",
        reason: ELLIPSIS,
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
        rule: Rule::BlitzUnsupported,
        selector: ".att",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".inv-desc.folded",
        reason: "a folded invitation shows three lines; the webview clamps them, Phase B clips by characters",
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".inv-answered .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".hc .flag .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".hc .flag.info .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".linkpill",
        reason: ELLIPSIS,
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
        rule: Rule::FocusPseudoClass,
        selector: ".book-find:focus-within",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".book-row .who b",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".book-row .addr, .book-row .origin, .hc .contact .origin",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".book-row:hover .acts, .book-row:focus-within .acts",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".rules-text b",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".rules-when",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".rules-row:hover .rules-row-acts, .rules-row:focus-within .rules-row-acts",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".rules-action .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".keys-text b",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".keys-row:hover .keys-row-acts, .keys-row:focus-within .keys-row-acts",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::RawFontSize,
        selector: ".mono",
        reason: "a mono run is sized against the text it sits in, which no absolute token can follow",
    },
];

pub(in crate::ui) const MARKUP: &[Exception] = &[
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.fold-name",
        reason: IN_SUMMARY,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.more",
        reason: IN_SUMMARY,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.scrim",
        reason: "the peek's scrim sits in the window's grid under the peeked reader; quire's `Scrim` floats in the overlay host, over everything the window draws, and would cover the reader it dims around (a gap, reported)",
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.rq-head",
        reason: "a quoted message's head in the composer is a glyph, who and when, and a quieter hint, in two faces; quire's `Button` draws one label (a gap, reported)",
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "textarea.c-wire",
        reason: WIRE,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "span.av",
        reason: "an account's avatar wears the colour the account was given, which is data",
    },
];
