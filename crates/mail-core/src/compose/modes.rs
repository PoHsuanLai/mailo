//! The other ways to answer or pass on a message: a reply that quotes only what was selected, a
//! reply to the list a message came through, a redirect, and a new message made from an old one.
//!
//! Each is a draft (or, for a redirect, a submission) built here, where replying and forwarding
//! are built, so the window and anything else that asks get the same one. None of them is a new
//! kind of draft: a reply to the list is a reply addressed elsewhere, a selection reply is a
//! reply quoting less, and Edit as New is a blank draft filled in. Only a redirect is new, and it
//! is no draft at all: the original goes again as it is, so there is nothing to edit.

use super::enclosed::sent_bytes_of;
use super::{carried, identity_addressed, identity_of, quoted, quoting, save, signed};
use crate::error::CoreError;
use chrono::{DateTime, Local, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// Whom an answer goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// A reply to the sender, or to everyone, as [`Draft::reply_to`] addresses it.
    Scope(ReplyScope),
    /// A reply to the list the message came through, at the address its `List-Post` names
    /// (RFC 2369 §3.4), and to nobody else: the person who wrote to the list reads it there.
    List,
}

/// What an answer quotes beneath its attribution line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote<'a> {
    /// The original's whole text part.
    Whole,
    /// Only this, the text the person had selected in the original when they asked to reply.
    /// Blank is [`Quote::Whole`]: a selection of nothing but space is a click, not a choice.
    Selection(&'a str),
}

/// [`draft_answer_in`], with the attribution line written in this machine's zone.
pub fn draft_answer(
    store: &SqliteStore,
    message: MessageId,
    to: Answer,
    quote: Quote<'_>,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    draft_answer_in(store, message, to, quote, "", now, &Local)
}

/// Create and persist an answer to `message`: addressed `to`, `body` (signed) above the quote,
/// and `quote` of the original beneath its attribution line.
///
/// The attribution line is written in `zone`, named rather than read from the machine because
/// the line leaves this machine: a test that let `Local` decide would assert whatever zone the
/// runner happens to be in. A reply to the list is refused, with
/// [`CoreError::NoListPost`], for a message that names no list address; one whose body has not
/// been fetched cannot say, and is refused as not fetched.
pub fn draft_answer_in<Tz: chrono::TimeZone>(
    store: &SqliteStore,
    message: MessageId,
    to: Answer,
    quote: Quote<'_>,
    body: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Draft, CoreError>
where
    Tz::Offset: std::fmt::Display,
{
    let original = store.message(message)?;
    let identity = identity_of(store, original.account.clone(), None)?;

    let mut draft = match to {
        Answer::Scope(scope) => Draft::reply_to(&original, &identity, scope, now),
        Answer::List => {
            let post = list_post(store, &original)?;
            // A reply in every other respect, its subject and threading among them: only who
            // reads it changes.
            let mut draft = Draft::reply_to(&original, &identity, ReplyScope::Sender, now);
            draft.to = post;
            draft.cc.clear();
            draft.bcc.clear();
            draft
        }
    };
    // `Draft::reply_to` leaves the text empty on purpose — quoting is a rendering decision, not
    // a property of the draft — so the quoting happens here, where the renderer is.
    let body = signed(body, &identity);
    draft.text = match quote {
        Quote::Selection(selected) if !selected.trim().is_empty() => {
            quoting(&body, &original, Some(selected.trim_end()), zone)
        }
        Quote::Whole | Quote::Selection(_) => quoted(&body, &original, zone),
    };
    save(store, &draft)?;
    Ok(draft)
}

/// Where a message to `message`'s list goes: its `List-Post` addresses, or
/// [`CoreError::NoListPost`] when it names none.
///
/// Reads the stored message's header block, which is where the field is: nothing keeps it in a
/// column, for the reason [`crate::unsubscribe`] gives for its own list headers.
pub fn list_post(store: &SqliteStore, message: &Message) -> Result<Vec<Address>, CoreError> {
    let post = crate::unsubscribe::list_of(store, message)?.post;
    if post.is_empty() {
        return Err(CoreError::NoListPost);
    }
    Ok(post)
}

/// Create and persist a new message made from `message`: Edit as New, and Send Again.
///
/// Its recipients (blind ones too, where the stored copy names them, as a Sent copy does), its
/// subject as it was, its text, and the files it carried that are held here, within
/// [`super::ATTACHMENT_BUDGET`]. Not a reply and not a forward: it answers nothing and threads
/// under nothing, and its text is not signed again, because a message of the person's own already
/// ends with the signature it went with.
///
/// It leaves from the address the original was from, when that is one of the account's own (Send
/// Again of something sent), else from the one it reached, else the default.
pub fn draft_again(
    store: &SqliteStore,
    message: MessageId,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let original = store.message(message)?;
    let account = original.account.clone();
    let wanted = identity_addressed(store, account.clone(), std::slice::from_ref(&original.from))
        .or_else(|| {
            let reached: Vec<Address> = original.to.iter().chain(&original.cc).cloned().collect();
            identity_addressed(store, account.clone(), &reached)
        });
    let identity = identity_of(store, account, wanted)?;

    let mut draft = Draft::blank(&identity, now);
    draft.to = original.to.clone();
    draft.cc = original.cc.clone();
    draft.bcc = original.bcc.clone();
    draft.subject = original.subject.clone();
    if let Body::Present {
        text: Some(text), ..
    } = &original.body
    {
        draft.text = text.clone();
    }
    draft.attachments = carried(&original.attachments);
    save(store, &draft)?;
    Ok(draft)
}

/// Redirect `message` to `to`: queue the original, unchanged, beneath a `Resent-` block
/// ([`mail_mime::resent`]), and say whom it went to.
///
/// Sent from the address the message reached, else the account's default, through the ordinary
/// outbox, so it leaves on the next sync like any message and is retried like one. There is no
/// draft: nothing in it is the person's to edit. The outbox's entry is keyed by an id no draft
/// has, which the drain already treats as nothing to mark (as a read receipt's is).
///
/// Refused, each with what to do instead, where it cannot go as it came: a body not downloaded,
/// one rebuilt from its parts, and an account that sends through Microsoft Graph, which delivers
/// to the people a message's headers name rather than to the envelope.
pub fn redirect(
    store: &SqliteStore,
    message: MessageId,
    to: &[Address],
    now: DateTime<Utc>,
) -> Result<Vec<String>, CoreError> {
    let original = store.message(message)?;
    let account = original.account.clone();
    if crate::sync::local_accounts(store).contains(&account) {
        return Err(mail_runtime::RuntimeError::NoServer("send from").into());
    }
    // This account's plan alone: another's that will not read is no reason to refuse this.
    let graph = store.list_accounts()?.into_iter().find(|stored| {
        stored.id == account
            && matches!(
                &stored.plan,
                Ok(AccountPlan {
                    outgoing: Outgoing::Graph,
                    ..
                })
            )
    });
    if let Some(stored) = graph {
        return Err(CoreError::RedirectThroughGraph {
            address: stored.address,
        });
    }
    let bytes = sent_bytes_of(store, &original).map_err(|why| match why {
        CoreError::NotDownloaded => CoreError::NothingToRedirect,
        CoreError::RebuiltFromParts => CoreError::RebuiltNotRedirected,
        other => other,
    })?;
    let reached: Vec<Address> = original.to.iter().chain(&original.cc).cloned().collect();
    let identity = identity_of(
        store,
        account.clone(),
        identity_addressed(store, account.clone(), &reached),
    )?;
    let id = DraftId::generate();
    let post = mail_mime::resent(
        &bytes,
        &mail_mime::Resending {
            from: &identity.from,
            to,
            at: now,
            id,
        },
    )?;
    let raw = store.blobs().put(&post.message)?;
    let queued = store.enqueue(
        account,
        RemoteIntent::Send {
            draft: id,
            raw,
            mail_from: post.mail_from,
            rcpt_to: post.rcpt_to.clone(),
        },
        // Nothing to undo: the outbox cannot take back bytes once they are on the wire.
        &Patch {
            id: ChangeId::generate(),
            changes: Vec::new(),
        },
        now,
    )?;
    if queued.is_none() {
        return Err(CoreError::SubmissionNotQueued);
    }
    Ok(post.rcpt_to)
}
