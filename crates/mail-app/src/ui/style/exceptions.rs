//! What mailo's own CSS and markup still may not do by quire's lint, and why. Each entry is
//! one selector, compared whole; a reason names why the rule is right for mailo as it stands,
//! and a gap in quire is reported to quire (coherence rule 3, FINDINGS "quire requests"), never
//! patched here.
//!
//! quire v0.2.21 put a short-labelled Setting row's control at the row's end, which was the line
//! that stood here before.

use ds_lint::{Exception, Rule};

pub(super) const STYLE: &[Exception] = &[Exception {
    rule: Rule::BlitzUnsupported,
    selector: ".truncate",
    reason: "the Blitz mailo pins (0be21979, since e7f9f877) draws `text-overflow: ellipsis` on a \
             clipping nowrap block; the lint predates it (a quire request). `.ds-truncate`'s fade \
             would dim the end of a name that fits, where its box is as wide as its text.",
}];

pub(in crate::ui) const MARKUP: &[Exception] = &[];
