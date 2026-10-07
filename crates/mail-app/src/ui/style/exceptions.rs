//! What mailo's own CSS and markup still may not do by quire's lint, and why. Each entry is
//! one selector, compared whole; a reason names why the rule is right for mailo as it stands,
//! and a gap in quire is reported to quire (coherence rule 3, FINDINGS "quire requests"), never
//! patched here.
//!
//! One at present, until the re-pin to quire v0.2.21 drops it.

use ds_lint::{Exception, Rule};

pub(super) const STYLE: &[Exception] = &[Exception {
    rule: Rule::DsInternals,
    selector: ".ds-field-row[*|data-layout=setting] > .ds-field-row-control",
    reason: "quire v0.2.20 left a short-labelled Setting row's control mid-row; reported and fixed \
             in v0.2.21, whose re-pin removes this line",
}];

pub(in crate::ui) const MARKUP: &[Exception] = &[];
