//! Personal information beside mail: vCards, and the WebDAV conversation CardDAV is spoken in.
//!
//! Pure, like `mail-mime`: text in, values out, and text back. The requests are built and the
//! replies are read here; sending them is `mail-runtime`'s business. Kept apart from `mail-mime`
//! because none of it is MIME, and apart from `mail-proto` because none of it is a machine.
//!
//! [`line`] is the content-line grammar vCard (RFC 6350 §3.3) shares with iCalendar (RFC 5545
//! §3.1), so a calendar parser can stand on it without a second reading of folding and quoting.

pub mod dav;
pub mod error;
pub mod line;
pub mod vcard;

pub use dav::{Multistatus, Props, Resource, Response};
pub use error::PimError;
pub use line::{ContentLine, Param};
pub use vcard::{Card, Email, Name, Phone, Version};
