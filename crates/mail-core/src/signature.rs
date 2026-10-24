//! Signatures: several to an identity, kept in Settings, picked in the composer, and changed
//! with From.
//!
//! The store keeps them (`mail_domain::Signature`); this is what the window and the command line
//! do with them. A signature is written in the composer's editor, so it is kept as the editor's
//! HTML and as plain text from the same document, and a message is signed with both: the text
//! in its text part, the HTML in its HTML part. [`doc`] is the signature inside a document.

mod doc;

pub use doc::{Placement, nodes_of, plain_of, recognise, region, swap};

use crate::compose::{doc_of, identity_of, save};
use crate::editor::{Doc, apply, to_flowed, to_html};
use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::{
    Draft, DraftId, Identity, IdentityId, IsDefault, Signature, SignatureId, SignatureUse,
    UNNAMED_SIGNATURE,
};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// The identity's signatures, oldest first.
pub fn of_identity(store: &SqliteStore, identity: IdentityId) -> Result<Vec<Signature>, CoreError> {
    Ok(store.signatures(identity)?)
}

/// The identity's default for `purpose`, if it has one that says something.
pub fn default_for(
    store: &SqliteStore,
    identity: IdentityId,
    purpose: SignatureUse,
) -> Result<Option<Signature>, CoreError> {
    Ok(store
        .default_signature(identity, purpose)?
        .filter(|signature| !signature.is_blank()))
}

/// Which default a draft starts with: a reply's or a forward's, or a new message's.
pub fn purpose_of(draft: &Draft) -> SignatureUse {
    if draft.in_reply_to.is_some() || draft.forward_of.is_some() {
        SignatureUse::Replies
    } else {
        SignatureUse::NewMail
    }
}

/// Every identity of every account that sends, by account oldest first, with its signatures:
/// what Settings lists.
pub fn every(store: &SqliteStore) -> Result<Vec<(Identity, Vec<Signature>)>, CoreError> {
    let mut out = Vec::new();
    for (_, account) in crate::compose::sending_accounts(store) {
        for identity in store.identities(account)? {
            let signatures = store.signatures(identity.id)?;
            out.push((identity, signatures));
        }
    }
    Ok(out)
}

/// A new, empty signature for `identity` called `name`, or [`UNNAMED_SIGNATURE`] when the name
/// is blank. The identity's first is its default for new mail and for replies, as the one
/// signature an identity had always was.
pub fn create(
    store: &SqliteStore,
    identity: IdentityId,
    name: &str,
) -> Result<Signature, CoreError> {
    let first = store.signatures(identity)?.is_empty();
    let default = if first {
        IsDefault::Default
    } else {
        IsDefault::Alternate
    };
    let signature = Signature {
        id: SignatureId::generate(),
        identity,
        name: named(name),
        html: None,
        text: String::new(),
        new_mail: default,
        replies: default,
    };
    store.put_signature(&signature)?;
    Ok(signature)
}

/// Keep `doc`, as the composer's editor wrote it, as the signature's words: its HTML and its
/// plain text. A document with nothing in it keeps no HTML.
pub fn write(store: &SqliteStore, id: SignatureId, doc: &Doc) -> Result<Signature, CoreError> {
    let text = plain_of(&doc.nodes);
    let html = (!text.trim().is_empty()).then(|| to_html(doc));
    let signature = Signature {
        html,
        text,
        ..store.signature(id)?
    };
    store.put_signature(&signature)?;
    Ok(signature)
}

/// Call the signature `name`, or [`UNNAMED_SIGNATURE`] when the name is blank.
pub fn rename(store: &SqliteStore, id: SignatureId, name: &str) -> Result<Signature, CoreError> {
    let signature = Signature {
        name: named(name),
        ..store.signature(id)?
    };
    store.put_signature(&signature)?;
    Ok(signature)
}

/// Make the signature its identity's default for `purpose`, or stop it being, with
/// [`IsDefault::Alternate`]. The default it replaces stops being one.
pub fn make_default(
    store: &SqliteStore,
    id: SignatureId,
    purpose: SignatureUse,
    default: IsDefault,
) -> Result<Signature, CoreError> {
    let had = store.signature(id)?;
    let signature = match purpose {
        SignatureUse::NewMail => Signature {
            new_mail: default,
            ..had
        },
        SignatureUse::Replies => Signature {
            replies: default,
            ..had
        },
    };
    store.put_signature(&signature)?;
    Ok(signature)
}

/// Delete the signature, returning the name it had.
pub fn delete(store: &SqliteStore, id: SignatureId) -> Result<String, CoreError> {
    let had = store.signature(id)?;
    store.delete_signature(id)?;
    Ok(had.name)
}

/// The name a signature is kept under: what was typed, trimmed, or [`UNNAMED_SIGNATURE`].
fn named(name: &str) -> String {
    match name.trim() {
        "" => UNNAMED_SIGNATURE.to_owned(),
        name => name.to_owned(),
    }
}

/// `draft` with its signature made `next`, or taken off with `None`, its text and HTML written
/// again from the document; `draft` as it was when that changes nothing. `known` are the
/// signatures it may have now, which are its identity's.
pub fn resigned(
    draft: &Draft,
    known: &[Signature],
    next: Option<&Signature>,
    placement: Placement,
) -> Draft {
    let mut doc = doc_of(draft);
    let Some((op, _)) = swap(&doc, known, next, placement) else {
        return draft.clone();
    };
    if apply(&mut doc, op).is_err() {
        return draft.clone();
    }
    Draft {
        text: to_flowed(&doc),
        html: Some(to_html(&doc)),
        ..draft.clone()
    }
}

/// Send `draft` as `identity` from now on, with that identity's default signature in place of
/// the one it has: the change of From, at the store. A draft already on that identity is left
/// as it is.
///
/// `identity` may be on another account, which the draft then moves to: both columns move
/// together, for the reason [`crate::compose::move_draft_to`] gives.
pub fn send_as(
    store: &SqliteStore,
    draft: DraftId,
    account: AccountId,
    identity: Option<IdentityId>,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let had = store.draft(draft)?;
    let next = identity_of(store, account.clone(), identity)?;
    if had.identity == next.id {
        return Ok(had);
    }
    let purpose = purpose_of(&had);
    let known = store.signatures(had.identity)?;
    let signature = default_for(store, next.id, purpose)?;
    let moved = Draft {
        account,
        identity: next.id,
        updated: now,
        ..resigned(&had, &known, signature.as_ref(), Placement::default())
    };
    save(store, &moved)?;
    Ok(moved)
}

#[cfg(test)]
mod tests;
