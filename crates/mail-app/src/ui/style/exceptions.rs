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
/// A row's strip buttons, the selection bubble's marks and the frame's small words.
const STRIP_AND_FRAME: &str = "three kinds of raw button: a row's strip, since quire's `HoverStrip` calls a button's `onclick` only once it has measured it, so a press in a document with no layout does nothing (a gap, reported); the selection bubble's B, I, U and S, whose faces are the marks themselves, where `Button` takes a text label; and the sidebar's Clear, Show all and + New, set in the frame's ink, which no `Button` variant wears (a gap, reported)";
/// A button on the frame, in the frame's own ink.
const FRAME_INK: &str = "set on the Space's colour in the frame's ink (`--f-ink`); quire's `Button` variants all wear the card's ink (a gap, reported)";
/// An inline field.
const INLINE_FIELD: &str = "an inline field set in the face of where it sits (the composer's subject in the display face, a recipient beside its chips); quire's `TextInput` takes its face from its own sheet only (a gap, reported)";
/// The composer's wire.
const WIRE: &str = "the composer's wire is the textarea the editor glue reads the page through, by design, not a field anyone types in";
/// A Space's own colours, which are data.
const SPACE_COLOUR: &str =
    "a Space's colours are the person's own, drawn as they chose them: data, not a design value";

pub(super) const STYLE: &[Exception] = &[
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".space-name",
        reason: ELLIPSIS,
    },
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
        rule: Rule::FocusPseudoClass,
        selector: ".row:hover .strip, .row:focus-within .strip",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".row:hover .strip button, .row:focus-within .strip button",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::Important,
        selector: ".strip button:active",
        reason: "a pressed strip button must shrink under the pop-in that holds its transform with `forwards`",
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".fmenu .it .sc .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".fmenu .it .rm .ds-ic",
        reason: GLYPH_TINT,
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
        rule: Rule::BlitzUnsupported,
        selector: ".hc .msg p",
        reason: "a hover card quotes two lines of a message; the webview clamps them, Phase B clips by characters",
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
        rule: Rule::DsInternals,
        selector: ".pval.files-dest .ds-ic",
        reason: GLYPH_TINT,
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
        selector: "button",
        reason: STRIP_AND_FRAME,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.item",
        reason: "a place is dragged onto: its element hears the pointer enter and leave; quire's `SidebarItem` (`ItemKind::Place`) takes no pointer hooks and writes no `data-place` (a gap, reported)",
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.pin.acct",
        reason: "a local-folders account is on no provider, and quire's `AccountTile` always draws a `ProviderMark` (`AccountFace::One` takes a `Provider`, not an `Option`; a gap, reported)",
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.pval",
        reason: "a property value opens its dropdown anchored to itself and ends in a caret; quire's `Button` draws a label and a leading icon, no trailing mark (a gap, reported)",
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.rm",
        reason: "mailo's own menu (a toggle list that stays open as each row is picked, and a menu drawn inline in a card) keeps its row remove; quire's `Menu` closes on every pick and always floats (a gap, reported)",
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.space-name",
        reason: FRAME_INK,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.inline",
        reason: INLINE_FIELD,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.inline.c-title",
        reason: INLINE_FIELD,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.inline.pinput",
        reason: INLINE_FIELD,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "textarea.c-wire",
        reason: WIRE,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "button",
        reason: SPACE_COLOUR,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "span.av",
        reason: "an account's avatar wears the colour the account was given, which is data",
    },
    Exception {
        rule: Rule::HexColour,
        selector: "span.tile.round",
        reason: SPACE_COLOUR,
    },
];
