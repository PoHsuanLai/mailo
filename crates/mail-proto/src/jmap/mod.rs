//! JMAP (RFC 8620, the core; RFC 8621, mail) as values in and values out.
//!
//! JMAP is JSON over HTTPS, so there is no byte-level session to drive and no [`crate::Machine`]
//! here. What is protocol — which method to call with which arguments, how one call names the
//! result of another, what a response means and which refusals are worth retrying — is here, as
//! pure functions over [`serde_json::Value`]. Posting a request, following a download URL and
//! holding an event stream open is `mail-runtime`'s, the same split the Graph sender has.
//!
//! Hostile input is the rule, as everywhere in this crate: a response is a stranger's JSON, and
//! a field of the wrong type is a [`ProtoError::Malformed`] value, never a panic.
//!
//! [`ProtoError::Malformed`]: crate::ProtoError::Malformed

mod changes;
mod email;
mod error;
mod field;
mod mailbox;
mod push;
mod request;
mod response;
mod session;
mod set;
mod submit;
mod template;

pub use changes::{Changes, More, QueryPage, state_of};
pub use email::{EmailSummary, Filing, HasAttachment, SUMMARY, filing};
pub use error::MethodError;
pub use mailbox::{DELIMITER, JmapMailbox, JmapRole, Mailboxes};
pub use push::{Event, EventStream, StateChange};
pub use request::{
    Call, Ids, blob_ids, email_changes, email_get, email_import, email_query, email_set,
    identity_get, mailbox_changes, mailbox_get, mailbox_set, request, total_query,
};
pub use response::Responses;
pub use session::{Limits, Session, Submission};
pub use set::{
    SetError, SetResult, filing_patch, flags_patch, folder_work, keyword_patch, labels_patch,
};
pub use submit::{Filed, Identity, Sent, choose_identity, submission, submitted};
pub use template::expand;

/// The capability every request uses (RFC 8620 §2).
pub const CORE: &str = "urn:ietf:params:jmap:core";
/// Mail: mailboxes, threads and emails (RFC 8621 §1.3.1).
pub const MAIL: &str = "urn:ietf:params:jmap:mail";
/// Sending: identities and email submission (RFC 8621 §1.3.2).
pub const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
