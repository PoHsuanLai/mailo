//! Domain values to RFC 5322 bytes.

use crate::MimeError;
use mail_domain::{BlobId, Draft, Identity, Message};

/// Build the bytes to submit for `draft`.
///
/// `parts` supplies the contents of every [`mail_domain::PendingAttachment`] by `BlobId`;
/// a referenced blob that is absent is [`MimeError::MissingPart`] rather than a silent drop.
///
/// `in_reply_to` is the message being replied to, when there is one. It is needed for correct
/// `In-Reply-To` and `References` headers — threading is reconstructed by the *recipient's*
/// client from those, so getting them wrong breaks the conversation on their side, where we
/// will never see it.
pub fn build(
    _draft: &Draft,
    _identity: &Identity,
    _in_reply_to: Option<&Message>,
    _parts: &[(BlobId, Vec<u8>)],
) -> Result<Vec<u8>, MimeError> {
    todo!("wave 2")
}
