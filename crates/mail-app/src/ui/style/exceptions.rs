//! What mailo's own CSS and markup still may not do by quire's lint, and why. Each entry is
//! one selector, compared whole; a reason names why the rule is right for mailo as it stands,
//! and a gap in quire is reported to quire (coherence rule 3, FINDINGS "quire requests"), never
//! patched here.
//!
//! There is none at present: quire v0.2.21 puts a short-labelled Setting row's control at the
//! row's end, which was the one line that stood here.

use ds_lint::Exception;

pub(super) const STYLE: &[Exception] = &[];

pub(in crate::ui) const MARKUP: &[Exception] = &[];
