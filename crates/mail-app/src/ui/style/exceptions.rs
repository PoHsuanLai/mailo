//! What mailo's own CSS and markup still may not do by quire's lint, and why. Each entry is
//! one selector, compared whole; a reason names why the rule is right for mailo as it stands,
//! and a gap in quire is reported to quire (coherence rule 3, FINDINGS "quire requests"), never
//! patched here.

use ds_lint::{Exception, Rule};

pub(super) const STYLE: &[Exception] = &[Exception {
    rule: Rule::DsInternals,
    selector: ".c-props .ds-field-row-control",
    reason: "quire's FieldRow lays its control cell out as one control; the composer's To, Cc, \
             Attached and Sends rows hold several (chips, a field, a button) that wrap with a \
             gap. `FieldRow` needs a wrapping control cell (quire request), and this line goes \
             when it has one",
}];

pub(in crate::ui) const MARKUP: &[Exception] = &[];
