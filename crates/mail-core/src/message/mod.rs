//! A message as the reader shows it: the body rendered for its sandboxed frame, and the cache of
//! those renderings the front-end owns; and what the reader finds out about one message beyond
//! its body (its checks, an invitation, a read receipt, a list's way out, OpenPGP or S/MIME),
//! with the caches of those answers in a [`Looks`] the front-end owns.
//!
//! No window is in here. What is drawn around the frame, and the words, are the front-end's.

mod frame;
mod frames;
mod list;
mod looks;
mod seal;

pub use frame::{FrameBody, Source, render};
pub use frames::{FIRST_SCREEN, Frames, Key, ON_THE_FRAME, Sent, Unsealed, ahead};
pub use list::{Offer, offer_of};
pub use looks::{Bodies, Invited, Looks, Read, Receipt};
pub use seal::{Opening, Protection, Scheme, Sealed, Tried};
