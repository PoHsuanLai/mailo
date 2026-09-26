//! Personal information beside mail: vCards, calendar invitations, and the WebDAV conversation
//! CardDAV is spoken in.
//!
//! Pure, like `mail-mime`: text in, values out, and text back. The requests are built and the
//! replies are read here; sending them is `mail-runtime`'s business. Kept apart from `mail-mime`
//! because none of it is MIME, and apart from `mail-proto` because none of it is a machine.
//!
//! [`line`] is the content-line grammar vCard (RFC 6350 §3.3) shares with iCalendar (RFC 5545
//! §3.1); [`vcard`] and [`ical`] both stand on it.

pub mod dav;
pub mod error;
pub mod ical;
pub mod invite;
pub mod line;
pub mod vcard;

pub use dav::{Multistatus, Props, Resource, Response};
pub use error::PimError;
pub use ical::Calendar;
pub use invite::{Invite, Kind, Me, Revision, Unplaced, When, WhenShown, show_when, summarise};
pub use line::{ContentLine, Param};
pub use vcard::{Card, CardKind, Email, Member, Name, Phone, Version};
