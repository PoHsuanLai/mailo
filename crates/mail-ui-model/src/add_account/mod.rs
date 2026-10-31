//! What the Add account window does without drawing: porter's account service hosted in this
//! process, with the mail providers it is offered, and the add that follows a sign-in.
//!
//! The window that draws the sheet (`mail-ui`'s `add_account`) holds the rest: its mapping to
//! quire's parts, its size and the direct sign-ins.

pub mod host;
pub mod provider;

#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod provider_tests;
