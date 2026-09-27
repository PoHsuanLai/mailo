//! Brand Indicators for Message Identification: whether a message may show its sender's logo,
//! read from what DNS and the receiving server said. Pure: the lookups and fetches are
//! `mail_runtime::bimi`'s, and drawing the logo is theirs too.
//!
//! draft-brand-indicators-for-message-identification: a domain publishes `default._bimi.<domain>`
//! naming an SVG and the certificate that vouches for it; a receiver shows the logo only for mail
//! that passed DMARC under a policy at enforcement. This client adds one rule of its own: no
//! logo without the certificate. A self-asserted logo is a picture anybody with a domain can
//! publish, which is the phishing it would otherwise help.

mod record;
pub mod vmc;

pub use record::{
    BimiRecord, Disposition, DmarcRecord, RecordError, dmarc_passed_for, enforced, parse_dmarc,
    parse_record, pick_dmarc, pick_record,
};
pub use vmc::{MarkProblem, same_logo, svg_bytes, verified_logo};
