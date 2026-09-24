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
/// Stacking inside the Space editor.
const EDITOR_LAYER: &str = "orders the Space editor's handle and footer inside the editor's own stacking context; no design layer names a sheet's inner order";
/// Every raw form control, for one reason.
const RAW_CONTROL: &str = "quire's `Button` has no `title`, `aria-label` or `aria-expanded`, and `TextInput` no `onfocus`/`onblur` and no range or password kind (a reported quire gap); every raw control waits on it";
/// The composer's wire.
const WIRE: &str = "the composer's wire is the textarea the editor glue reads the page through, by design, not a field anyone types in";
/// A Space's own colours, which are data.
const SPACE_COLOUR: &str =
    "a Space's colours are the person's own, drawn as they chose them: data, not a design value";

pub(super) const STYLE: &[Exception] = &[
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".app:focus, .app:focus-visible",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".pin.acct[*|aria-pressed=\"false\"] .av:not(.all)",
        reason: "an account not in view is desaturated; quire has no muted-avatar tone yet (reported), and the webview draws the filter",
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".today-item .t",
        reason: ELLIPSIS,
    },
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
        rule: Rule::RawZIndex,
        selector: ".handle",
        reason: EDITOR_LAYER,
    },
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".handle:focus-visible",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".ed-foot",
        reason: "the Space editor's footer sticks to the sheet's bottom while it scrolls, which the webview draws; Phase B keeps it outside the scroller",
    },
    Exception {
        rule: Rule::RawZIndex,
        selector: ".ed-foot",
        reason: EDITOR_LAYER,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".nm",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".row-sub",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".row-snip",
        reason: ELLIPSIS,
    },
    Exception {
        rule: Rule::FocusPseudoClass,
        selector: ".row:hover .star, .star[*|data-on=\"true\"], .star:focus-visible",
        reason: FOCUS,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".star .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".star[*|data-on=\"true\"] .ds-ic",
        reason: GLYPH_TINT,
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
        rule: Rule::Keyframes,
        selector: "@keyframes cmdk-rise",
        reason: "mailo's sheets rise without fading, so they are opaque on their first frame; quire's `rise` and `cmdk-in` fade (a gap, reported)",
    },
    Exception {
        rule: Rule::BlitzUnsupported,
        selector: ".cmdk .snip",
        reason: ELLIPSIS,
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
        rule: Rule::Keyframes,
        selector: "@keyframes fade-in",
        reason: "the scrim settles at .16; quire's `fade` runs to 1 (a gap, reported)",
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
        rule: Rule::FontFamily,
        selector: ".bubble .serif",
        reason: "the selection bubble's Serif button shows the serif a message will be written in; quire has no serif face token (reported)",
    },
    Exception {
        rule: Rule::DsInternals,
        selector: ".c-warn .ds-ic",
        reason: GLYPH_TINT,
    },
    Exception {
        rule: Rule::Keyframes,
        selector: "@keyframes pill-up",
        reason: "the send pill and mailo's own toast rise from below a `translateX(-50%)` centre; quire has no such entrance (a gap, reported)",
    },
    Exception {
        rule: Rule::SvgPaintInCss,
        selector: ".sendpill circle",
        reason: "the send pill's countdown ring is mailo's own vector until quire's SendPill carries an outbox's states (the markup lint names it too)",
    },
    Exception {
        rule: Rule::RawDuration,
        selector: ".sendpill .run.countdown",
        reason: "the ring drains over the send's grace period, the outbox's five seconds, which no motion level may shorten",
    },
    Exception {
        rule: Rule::Keyframes,
        selector: "@keyframes ring-drain",
        reason: "the send pill's countdown drains its ring's stroke; quire has no countdown keyframe (a gap, reported)",
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
        rule: Rule::Keyframes,
        selector: "@keyframes busy",
        reason: "quire's `breathe` fades the sync halo to nothing; a busy account's words must stay legible (a gap, reported)",
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
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.btn",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.item",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.marks-refresh",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.mini",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.mini.primary",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.pin.acct",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.pin.acct.acct-add",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.pval",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.rm",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.space-name",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.star",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "button.x",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.c-file",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.ed-name",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.inline",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.inline.c-title",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.inline.pinput",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.range",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "input.inp.search",
        reason: RAW_CONTROL,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "textarea.c-wire",
        reason: WIRE,
    },
    Exception {
        rule: Rule::RawMarkup,
        selector: "svg.field-dots",
        reason: "the Space editor's field of dots is a picture of the Space's colour field, drawn by the editor, not a glyph",
    },
    Exception {
        rule: Rule::HexColour,
        selector: "button",
        reason: SPACE_COLOUR,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "button.ds-space-dot",
        reason: "quire's own `SpaceDot` writes its gradient as an inline `background`, not a custom property (reported)",
    },
    Exception {
        rule: Rule::HexColour,
        selector: "div.handle",
        reason: SPACE_COLOUR,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "div.handle.on",
        reason: SPACE_COLOUR,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "i",
        reason: SPACE_COLOUR,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "span.av",
        reason: "an account's avatar wears the colour the account was given, which is data",
    },
    Exception {
        rule: Rule::HexColour,
        selector: "span.sw",
        reason: SPACE_COLOUR,
    },
    Exception {
        rule: Rule::HexColour,
        selector: "span.tile.round",
        reason: SPACE_COLOUR,
    },
];
