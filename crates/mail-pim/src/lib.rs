//! Personal information beside mail: vCards and calendar invitations.
//!
//! Pure, like `mail-mime`: text in, values out, and text back. Kept apart from `mail-mime`
//! because none of it is MIME, and apart from `mail-proto` because none of it is a machine.
//!
//! [`line`] is the content-line grammar vCard (RFC 6350 §3.3) shares with iCalendar (RFC 5545
//! §3.1); [`vcard`] and [`ical`] both stand on it.

pub mod error;
pub mod ical;
pub mod invite;
pub mod line;
pub mod vcard;

pub use error::PimError;
pub use ical::{Calendar, Party, Recurrence, RepeatEnd, RepeatUnit, Repeats};
pub use invite::{Invite, Kind, Me, Revision, Unplaced, When, summarise};
pub use line::{ContentLine, Param};
pub use vcard::{Card, CardKind, Email, Member, Name, Phone, Postal, Version};

/// The README's examples, compiled and run as doctests.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
