//! Composing from the command line: reply, send, and what is waiting.
//!
//! Split from [`crate::cli`] because these three are the only commands that *write* mail, and
//! because the interesting part is not the parsing — it is that "send" means "freeze the bytes
//! and queue them", never "open a connection". A send that depended on the network being up at
//! the moment the user pressed the key would lose the message on a train.

use crate::error::{CoreError, Logged, TimeError};
use crate::pgp::OpenPgp;
use crate::protect::Seal;
use crate::smime::Smime;
use chrono::{DateTime, Local, Utc};
use mail_domain::signing::{SigningKeyRef, SigningSecret};
use mail_domain::*;
use mail_editor::{FORWARDED, plain_to_flowed};
use mail_mime::posting;
use mail_net::Failure;
use mail_secrets::CredentialError;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

pub mod addresses;
mod doc;
mod enclosed;
mod format;
mod modes;
mod pictures;
mod submit;
mod weight;
pub use doc::{doc_from_text, doc_of};
pub use enclosed::{Carry, ENCLOSED, draft_forward_attached, eml_name, rebuilt};
pub use format::{Written, written_as};
pub use modes::{
    Answer, Quote, Redirected, Saying, draft_again, draft_answer, draft_answer_in, list_post,
    redirect, withdraw_redirect,
};
pub use pictures::{PictureSize, sized};
pub use weight::{
    Fit, Measure, Picture, Said, SendLimit, USUAL_LIMIT, Weight, can_ask_delivery_report, fit,
    send_limit, weigh,
};

use crate::signature::{self, Placement};

/// Read one identity, or the account's default, out of the `identities` table.
///
/// The table rather than `AccountPlan.identities`, and that choice matters: a draft's
/// `identity` column is a foreign key into this table, so any draft that exists at all has a
/// row here. The plan's copy is written at account creation and can go stale — an account added
/// before identities were created at setup has neither, but one restored from a plan alone
/// would have a list the table does not back. Reading the side the constraint enforces means
/// the lookup cannot disagree with the row that let the draft be saved.
pub fn identity_of(
    store: &SqliteStore,
    account: AccountId,
    wanted: Option<IdentityId>,
) -> Result<Identity, CoreError> {
    // The default first, and the id breaks the tie so an account with two alternates picks the
    // same one every time.
    let found = match wanted {
        Some(id) => store.identity(id),
        None => store
            .identities(account)
            .map(|identities| identities.into_iter().next()),
    };
    match found {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(match wanted {
            Some(id) => CoreError::DraftIdentityGone { id },
            None => CoreError::NoIdentityToSendAs,
        }),
        Err(e) => Err(CoreError::context("stored identity is unreadable", e)),
    }
}

/// Create and persist a reply to `message`, returning the draft.
///
/// The shared half of replying: the CLI formats the result as text and the shell opens a
/// composer on it. Both go through here, so the draft a window produces and the draft a command
/// produces are the same draft — including the quoting, which is the part most easily done two
/// different ways.
pub fn draft_reply(
    store: &SqliteStore,
    message: MessageId,
    scope: ReplyScope,
    body: &str,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    draft_reply_in(store, message, scope, body, now, &Local)
}

/// [`draft_reply`], its signature where `placement` puts it: the window's, which follows the
/// setting that moves it above the quoted original.
pub fn draft_reply_placed(
    store: &SqliteStore,
    message: MessageId,
    scope: ReplyScope,
    body: &str,
    now: DateTime<Utc>,
    placement: Placement,
) -> Result<Draft, CoreError> {
    reply_in(store, message, scope, body, now, &Local, placement)
}

/// The same, with the zone the attribution line is written in named.
///
/// Named rather than read from the machine, because the attribution line leaves this machine:
/// a test that let `Local` decide would assert whatever zone the test runner happens to be in,
/// and on a runner set to UTC that is the bug passing.
pub fn draft_reply_in<Tz: chrono::TimeZone>(
    store: &SqliteStore,
    message: MessageId,
    scope: ReplyScope,
    body: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Draft, CoreError>
where
    Tz::Offset: std::fmt::Display,
{
    reply_in(store, message, scope, body, now, zone, Placement::default())
}

/// A reply, its attribution written in `zone` and its signature where `placement` puts it.
///
/// The signature is the identity's default for replies, as text: the composer that opens the
/// draft puts the signature's own formatting back (`crate::signature::recognise`).
fn reply_in<Tz: chrono::TimeZone>(
    store: &SqliteStore,
    message: MessageId,
    scope: ReplyScope,
    body: &str,
    now: DateTime<Utc>,
    zone: &Tz,
    placement: Placement,
) -> Result<Draft, CoreError>
where
    Tz::Offset: std::fmt::Display,
{
    draft_answer_in(
        store,
        message,
        Saying {
            to: Answer::Scope(scope),
            quote: Quote::Whole,
            body,
        },
        now,
        zone,
        placement,
    )
}

/// Create and persist a forward of `message`, addressed to `to`.
///
/// `Draft::forward_of` in `mail-domain` gives the shape; this supplies the half
/// `Draft::reply_to` also leaves to the caller, the rendering of the message being carried.
///
/// Recipients are a parameter rather than derived: a forward is *to* someone, and there is no
/// answer in the original that is not a guess. `Draft::forward_of` leaves them empty for that
/// reason and this fills them in from what the user said.
pub fn draft_forward(
    store: &SqliteStore,
    message: MessageId,
    to: &[Address],
    body: &str,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    draft_forward_in(store, message, to, body, now, &Local)
}

/// The same, with the zone the forwarded header block is written in named.
///
/// Named for the reason [`draft_reply_in`] names it: the block leaves this machine, and a
/// function that reads the machine's zone can only be tested against whatever that machine is.
pub fn draft_forward_in<Tz: chrono::TimeZone>(
    store: &SqliteStore,
    message: MessageId,
    to: &[Address],
    body: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Draft, CoreError>
where
    Tz::Offset: std::fmt::Display,
{
    let draft = forward_unsaved_in(store, message, to, body, now, zone)?;
    save(store, &draft)?;
    Ok(draft)
}

/// The forward [`draft_forward_in`] makes, not saved: what a preview shows of one before it
/// exists.
pub fn forward_unsaved_in<Tz: chrono::TimeZone>(
    store: &SqliteStore,
    message: MessageId,
    to: &[Address],
    body: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Draft, CoreError>
where
    Tz::Offset: std::fmt::Display,
{
    let original = store.message(message)?;
    let identity = identity_of(store, original.account.clone(), answering(store, &original))?;
    // A forward is signed as a reply is, and above the forwarded block whatever a reply does:
    // the block is the message passed on, not a quotation under it.
    let signature = signature::default_for(store, identity.id, SignatureUse::Replies)?;

    let mut draft = Draft::forward_of(&original, &identity, now);
    draft.to = to.to_vec();
    draft.text = plain_to_flowed(&forwarded(
        &signed(body, signature.as_ref()),
        &original,
        zone,
    ));
    draft.attachments = carried(&original.attachments);
    Ok(draft)
}

/// The files a forward passes on: `original`'s attachments that are held here, in order, while
/// they fit in [`ATTACHMENT_BUDGET`].
///
/// A picture the HTML embeds by `cid:` is not one, since the forward goes as text. A part still
/// on the server is left behind, as is one past the budget: the forward is still worth sending
/// without it, and the composer shows what it carries.
///
/// Only a part kept counts against the budget: one left behind takes nothing of it, so a smaller
/// part after it still goes.
fn carried(attachments: &[Attachment]) -> Vec<PendingAttachment> {
    let mut total = 0u64;
    attachments
        .iter()
        .filter(|a| a.inline == Inline::Attached)
        .filter_map(|a| {
            let blob = a.blob()?;
            let with = total.saturating_add(a.size);
            (with <= ATTACHMENT_BUDGET).then(|| {
                total = with;
                PendingAttachment {
                    name: a.name.clone(),
                    mime: a.mime.clone(),
                    blob,
                    cid: None,
                }
            })
        })
        .collect()
}

/// Every account that can send, as `(address, id)`, oldest first.
///
/// Oldest first rather than alphabetical so the list does not reorder itself when an account is
/// added — a picker whose first entry moves is a picker that sends from the wrong address.
///
/// Local folders are not one: they have no server to send through, so they are never offered
/// as a From, and a new message never starts on them.
pub fn sending_accounts(store: &SqliteStore) -> Vec<(String, AccountId)> {
    let Some(accounts) = store
        .list_accounts()
        .or_log("the accounts to send from could not be read")
    else {
        return Vec::new();
    };
    // Read row by row rather than through `sync::configured`, which gives up on the whole list
    // when one plan does not parse: an account whose plan cannot be read is still offered, as it
    // always was, and only a plan that says Local is left out.
    let keeps_locally = |plan: &Result<AccountPlan, mail_store::StoreError>| {
        plan.as_ref()
            .is_ok_and(|plan| !plan.incoming.caps().has_server)
    };
    accounts
        .into_iter()
        .filter(|account| !keeps_locally(&account.plan))
        .map(|account| (account.address, account.id))
        .collect()
}

/// Which account a new message leaves from.
///
/// `wanted` is what the user said, matched on the address. With nothing said and one account
/// configured the answer is obvious; with nothing said and several, this refuses and lists them.
///
/// Refusing is the point. A reply inherits its account from the message it answers, so this is
/// the only place in the application where the sending address is a free choice — and a silent
/// default is a message that goes out from the wrong address, which is not a failure the sender
/// sees until someone replies to it.
pub fn account_for(store: &SqliteStore, wanted: Option<&str>) -> Result<AccountId, CoreError> {
    let accounts = sending_accounts(store);
    if accounts.is_empty() {
        return Err(CoreError::NoAccounts);
    }
    match wanted {
        Some(address) => accounts
            .iter()
            .find(|(had, _)| had.eq_ignore_ascii_case(address))
            .map(|(_, id)| id.clone())
            .ok_or_else(|| CoreError::NoSuchSender {
                address: address.to_owned(),
                known: accounts.iter().map(|(a, _)| a.clone()).collect(),
            }),
        None if accounts.len() == 1 => Ok(accounts[0].1.clone()),
        None => Err(CoreError::WhichAccount {
            known: accounts.iter().map(|(a, _)| a.clone()).collect(),
        }),
    }
}

/// Create and persist a message that answers nothing, returning the draft.
///
/// Mail written to someone who has not written first: a draft that is neither a reply nor a
/// forward.
///
/// Shaped like [`draft_reply`] deliberately — persisted immediately, returned whole — so the
/// composer opens on a draft that already exists and Discard has something to delete.
pub fn draft_new(
    store: &SqliteStore,
    account: AccountId,
    to: &[Address],
    subject: &str,
    body: &str,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let identity = identity_of(store, account, None)?;
    let mut draft = Draft::blank(&identity, now);
    draft.to = to.to_vec();
    draft.subject = subject.to_owned();
    // The signature, and nothing else. There is no original to quote and no header block to
    // write, which is the whole difference between this and the other two.
    let (text, html) = signed_new(store, &identity, body)?;
    draft.text = plain_to_flowed(&text);
    draft.html = html;
    save(store, &draft)?;
    Ok(draft)
}

/// Create and persist the message a `mailto:` link asks for (RFC 6068), returning the draft.
///
/// [`draft_new`] with the link's copies and blind copies too, and signed the same way: the
/// person clicked a link to write to someone, and writes the rest themselves. It is only a
/// draft. Nothing a link says sends it, and it is opened in the composer for the person to read
/// before anything goes anywhere, which is where a `bcc` the link added is seen.
pub fn draft_mailto(
    store: &SqliteStore,
    account: AccountId,
    link: &mail_mime::MailtoUri,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let identity = identity_of(store, account, None)?;
    let mut draft = Draft::blank(&identity, now);
    draft.to = link.to.clone();
    draft.cc = link.cc.clone();
    draft.bcc = link.bcc.clone();
    draft.subject = link.subject.clone();
    let (text, html) = signed_new(store, &identity, &link.body)?;
    draft.text = plain_to_flowed(&text);
    draft.html = html;
    save(store, &draft)?;
    Ok(draft)
}

/// Create and persist a message whose every word is given, returning the draft.
///
/// [`draft_new`] without the signature: for a message a program reads rather than a person, such
/// as the one a list's software takes as an unsubscribe, where a signature is noise in a body
/// that may be parsed as commands. `identity` picks the address it leaves from; `None` is the
/// account's default.
pub fn draft_exact(
    store: &SqliteStore,
    account: AccountId,
    identity: Option<IdentityId>,
    to: &[Address],
    subject: &str,
    body: &str,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let identity = identity_of(store, account, identity)?;
    let mut draft = Draft::blank(&identity, now);
    draft.to = to.to_vec();
    draft.subject = subject.to_owned();
    draft.text = plain_to_flowed(body);
    save(store, &draft)?;
    Ok(draft)
}

/// The address a message answering mail to `addressed` leaves from: the identity it reached,
/// else the account's default. What a `mailto:` unsubscribe is sent as, so the window can say it
/// before anything is sent.
pub fn address_addressed(
    store: &SqliteStore,
    account: AccountId,
    addressed: &[Address],
) -> Result<Address, CoreError> {
    identity_of(
        store,
        account.clone(),
        identity_addressed(store, account, addressed),
    )
    .map(|identity| identity.from)
}

/// The identity of `account` that one of `addressed` names, if any.
///
/// Which address a message reached is which address is on the list, and a list's software
/// takes off the address that writes to it — so an unsubscribe sent from the default identity,
/// for mail that came to an alias, would leave the alias subscribed.
pub fn identity_addressed(
    store: &SqliteStore,
    account: AccountId,
    addressed: &[Address],
) -> Option<IdentityId> {
    store
        .identities(account)
        .or_log("the account's identities could not be read")?
        .into_iter()
        .find(|identity| {
            addressed
                .iter()
                .any(|a| a.email.eq_ignore_ascii_case(&identity.from.email))
        })
        .map(|identity| identity.id)
}

/// The identity of `original`'s account that a reply to it, or a forward of it, leaves from:
/// the one that sent it, when it is the person's own, else the one it was addressed to; `None`,
/// the account's default, when it reached none of them by name (a list, a Bcc).
///
/// Mail that came to an alias is answered from the alias, as Apple Mail and Thunderbird answer
/// it: the person gave that address out, and an answer from another tells the correspondent an
/// address they were not given and puts the next message in a folder the alias's rules do not
/// see. The sender is asked first so that answering one's own sent message keeps its address.
pub(crate) fn answering(store: &SqliteStore, original: &Message) -> Option<IdentityId> {
    let own = store
        .identities(original.account.clone())
        .or_log("the account's identities could not be read")?;
    // In the order the original names them, so mail To an alias and Cc the account's own
    // address is answered from the alias it was written to.
    std::iter::once(&original.from)
        .chain(&original.to)
        .chain(&original.cc)
        .chain(&original.bcc)
        .find_map(|addressed| {
            own.iter()
                .find(|identity| identity.from.email.eq_ignore_ascii_case(&addressed.email))
        })
        .map(|identity| identity.id)
}

/// Every identity a message can leave from, by account oldest first and within an account its
/// own address first: what the composer's From offers.
///
/// Accounts as [`sending_accounts`] has them, so local folders are never one. An account whose
/// identities cannot be read offers none rather than hiding every other account's.
pub fn sending_identities(store: &SqliteStore) -> Vec<Identity> {
    sending_accounts(store)
        .into_iter()
        .flat_map(|(_, account)| {
            store
                .identities(account)
                .or_log_default("an account's identities could not be read")
        })
        .collect()
}

/// Move a draft to `identity`, and to its account: what picking an address in From does.
///
/// [`move_draft_to`]'s rule holds: the identity is read before anything is written, so one that
/// is gone leaves the draft where it was. A draft already from it is returned as it is.
pub fn move_draft_to_identity(
    store: &SqliteStore,
    draft: DraftId,
    identity: IdentityId,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let mut draft = store.draft(draft)?;
    if draft.identity == identity {
        return Ok(draft);
    }
    let Some(chosen) = store.identity(identity)? else {
        return Err(CoreError::DraftIdentityGone { id: identity });
    };
    // A copy to the address it was from ([`copy_myself`]) follows it to the new one: the person
    // wanted a copy, not a copy at the address they moved away from.
    if let Some(was) = store.identity(draft.identity)? {
        for list in [&mut draft.cc, &mut draft.bcc] {
            for address in list.iter_mut() {
                if address.email.eq_ignore_ascii_case(&was.from.email) {
                    *address = chosen.from.clone();
                }
            }
        }
    }
    draft.account = chosen.account;
    draft.identity = chosen.id;
    draft.updated = now;
    save(store, &draft)?;
    Ok(draft)
}

/// Where [`copy_myself`] puts the sender's own address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Myself {
    Cc,
    Bcc,
}

/// Put the address `draft` leaves from on its Cc or Bcc, so a copy of the message comes back to
/// the person who sent it, and save it. Nothing changes when the address is already on it.
pub fn copy_myself(store: &SqliteStore, draft: DraftId, to: Myself) -> Result<Draft, CoreError> {
    let mut draft = store.draft(draft)?;
    let identity = identity_of(store, draft.account.clone(), Some(draft.identity))?;
    let already = draft
        .to
        .iter()
        .chain(&draft.cc)
        .chain(&draft.bcc)
        .any(|a| a.email.eq_ignore_ascii_case(&identity.from.email));
    if already {
        return Ok(draft);
    }
    match to {
        Myself::Cc => draft.cc.push(identity.from),
        Myself::Bcc => draft.bcc.push(identity.from),
    }
    save(store, &draft)?;
    Ok(draft)
}

/// The most a single message may carry, before base64 expands it.
///
/// Encoding inflates by four bytes for every three, so this is about 27 MiB on the wire, which
/// is over the 25 MB Gmail and most of the rest refuse at. The check is here rather than at send
/// because the answer has to arrive while the file is being attached: a refusal at Send is one
/// that comes after the message is written, and there is nothing useful to do with it then.
pub const ATTACHMENT_BUDGET: u64 = 20 * 1024 * 1024;

/// What to call the content of a file, from its name.
///
/// A small table and `application/octet-stream` for everything else, rather than a dependency
/// that sniffs content. The receiving client re-sniffs anyway, the honest fallback is never
/// wrong — it says "bytes", which they are — and `mail_mime::build` replaces anything that is
/// not a well-formed media type regardless, so a wrong guess here cannot forge a header.
fn media_type_of(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    match extension.to_ascii_lowercase().as_str() {
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "json" => "application/json",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        // A saved message. As `message/rfc822` it is shown by the recipient's client as the
        // message it is, and `mail_mime::build` sends it under an encoding RFC 2046 allows.
        "eml" => ENCLOSED,
        _ => "application/octet-stream",
    }
}

/// Attach `bytes` to a draft under `name`.
///
/// Bytes rather than a path, because the window has a file chooser that hands over contents and
/// the command line has a path, and only one of those is I/O this function should be doing.
pub fn attach_bytes(
    store: &SqliteStore,
    draft: DraftId,
    name: &str,
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let mut draft = store.draft(draft)?;
    if on_its_way(&draft.state) {
        return Err(CoreError::DraftOnItsWay);
    }
    // The name a file arrives under is the name it goes out under, and it is about to be a
    // header. `safe_name` is the same function that decides where an *incoming* attachment may
    // be written, so a `../` or a newline is refused in both directions by one rule.
    let name = crate::attach::safe_name(name);

    let carried: u64 = attached_size(store, &draft);
    let total = carried.saturating_add(bytes.len() as u64);
    if total > ATTACHMENT_BUDGET {
        return Err(CoreError::AttachmentsTooBig {
            total: crate::attach::human_size(total),
            limit: crate::attach::human_size(ATTACHMENT_BUDGET),
        });
    }

    let blob = store
        .blobs()
        .put(bytes)
        .map_err(|e| CoreError::cannot(format!("store {name}"), e))?;
    draft.attachments.push(PendingAttachment {
        mime: media_type_of(&name).to_owned(),
        name,
        blob,
        cid: None,
    });
    draft.updated = now;
    save(store, &draft)?;
    Ok(draft)
}

/// Whether a file named `name` is a picture the body can show (`attach_inline` takes it): by
/// its name, as everything here that types a file does.
pub fn shows_inline(name: &str) -> bool {
    mail_mime::embeddable(media_type_of(name)).is_some()
}

/// Put the image `bytes`, named `name`, on a draft to be shown in its body, and say the
/// `Content-ID` the body refers to it by (`ImageRef::of_cid`).
///
/// It is stored as any attachment is, under the same budget, and marked with its `Content-ID`
/// so `mail_mime::build` sends it inside the HTML's `multipart/related` rather than as a file.
/// Only the types a reader draws inline are taken (`mail_mime::embeddable`: PNG, JPEG, GIF and
/// WebP, never SVG), and the pictures in one body together stay within what a draft reopens with
/// (`mail_mime::INLINE_BUDGET`), so one put in is never missing when the draft comes back.
pub fn attach_inline(
    store: &SqliteStore,
    draft: DraftId,
    name: &str,
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<(Draft, String), CoreError> {
    let mut draft = store.draft(draft)?;
    if on_its_way(&draft.state) {
        return Err(CoreError::DraftOnItsWay);
    }
    let name = crate::attach::safe_name(name);
    let Some(mime) = mail_mime::embeddable(media_type_of(&name)) else {
        return Err(CoreError::NotAnInlineImage { name });
    };

    let carried: u64 = attached_size(store, &draft);
    let total = carried.saturating_add(bytes.len() as u64);
    if total > ATTACHMENT_BUDGET {
        return Err(CoreError::AttachmentsTooBig {
            total: crate::attach::human_size(total),
            limit: crate::attach::human_size(ATTACHMENT_BUDGET),
        });
    }
    // Measured as the reopened draft spends it: as base64.
    let shown: u64 = draft
        .attachments
        .iter()
        .filter(|a| a.cid.is_some())
        .filter_map(|a| {
            store
                .blobs()
                .size(a.blob)
                .or_log("an inline image's size could not be read")
        })
        .chain([bytes.len() as u64])
        .map(|size| size.div_ceil(3) * 4)
        .sum();
    if shown > mail_mime::INLINE_BUDGET as u64 {
        return Err(CoreError::InlineImagesTooBig {
            limit: crate::attach::human_size(mail_mime::INLINE_BUDGET as u64 / 4 * 3),
        });
    }

    let blob = store
        .blobs()
        .put(bytes)
        .map_err(|e| CoreError::cannot(format!("store {name}"), e))?;
    let cid = (1..)
        .map(|n| format!("img{n}.{}@mailo", draft.id))
        .find(|cid| {
            !draft
                .attachments
                .iter()
                .any(|a| a.cid.as_ref() == Some(cid))
        })
        .unwrap_or_else(|| format!("img.{}@mailo", draft.id));
    draft.attachments.push(PendingAttachment {
        mime: mime.to_owned(),
        name,
        blob,
        cid: Some(cid.clone()),
    });
    draft.updated = now;
    save(store, &draft)?;
    Ok((draft, cid))
}

/// The images `draft` shows in its body, as the parts its stored HTML names them by: what the
/// composer reopens the body with. One the store no longer has is left out, and the body drops it.
pub fn inline_parts(store: &SqliteStore, draft: &Draft) -> Vec<mail_mime::ParsedPart> {
    draft
        .attachments
        .iter()
        .filter_map(|a| {
            let cid = a.cid.clone()?;
            let bytes = store
                .blobs()
                .get(a.blob)
                .or_log("an inline image could not be read")?;
            Some(mail_mime::ParsedPart {
                name: a.name.clone(),
                mime: a.mime.clone(),
                bytes,
                inline: mail_domain::Inline::Embedded { cid },
                remote: None,
            })
        })
        .collect()
}

/// Read `path` and attach it.
pub fn attach_file(
    store: &SqliteStore,
    draft: DraftId,
    path: &std::path::Path,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    // Checked before reading, so attaching a 4 GB file is an error rather than 4 GB of memory.
    let size = std::fs::metadata(path)
        .map_err(|e| CoreError::cannot(format!("read {}", path.display()), e))?
        .len();
    if size > ATTACHMENT_BUDGET {
        return Err(CoreError::FileTooBig {
            path: path.to_owned(),
            size: crate::attach::human_size(size),
            limit: crate::attach::human_size(ATTACHMENT_BUDGET),
        });
    }
    let bytes = std::fs::read(path)
        .map_err(|e| CoreError::cannot(format!("read {}", path.display()), e))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_owned());
    attach_bytes(store, draft, &name, &bytes, now)
}

/// Take attachment `index` back off a draft.
///
/// The blob stays. It is content-addressed and shared — the same bytes may be an attachment on
/// a message that already went out — so removing the reference is the whole of the operation.
pub fn detach(
    store: &SqliteStore,
    draft: DraftId,
    index: usize,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let mut draft = store.draft(draft)?;
    if index >= draft.attachments.len() {
        return Err(CoreError::NoDraftAttachment {
            have: draft.attachments.len(),
            index,
        });
    }
    draft.attachments.remove(index);
    draft.updated = now;
    save(store, &draft)?;
    Ok(draft)
}

/// The bytes a draft is already carrying.
///
/// `size` rather than `get`: measuring by reading would load every file already attached each
/// time another one is added, which on a message with three large files is three reads to
/// answer a question one column already holds.
fn attached_size(store: &SqliteStore, draft: &Draft) -> u64 {
    draft
        .attachments
        .iter()
        .filter_map(|a| {
            store
                .blobs()
                .size(a.blob)
                .or_log("an attachment's size could not be read")
        })
        .sum()
}

/// What a draft carries, as `(name, size)` ready to show.
///
/// The sizes come from the store rather than from whatever was just handed over, so a file that
/// failed to store cannot appear in the list as though it had.
pub fn attached_to(store: &SqliteStore, draft: &Draft) -> Vec<(String, String)> {
    // An image shown in the body is in the body, not in the list of files.
    draft
        .attachments
        .iter()
        .filter(|a| a.cid.is_none())
        .map(|a| {
            let size = store
                .blobs()
                .size(a.blob)
                .map(crate::attach::human_size)
                .unwrap_or_else(|_| "missing".to_owned());
            (a.name.clone(), size)
        })
        .collect()
}

/// One file on a draft, as [`attachments_of`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedFile {
    pub name: String,
    pub mime: String,
    /// `None` when the store no longer has the bytes, which a listing says rather than hides.
    pub size: Option<u64>,
}

/// What a draft is carrying, in the order they are numbered for [`detach`].
pub fn attachments_of(store: &SqliteStore, draft: DraftId) -> Result<Vec<AttachedFile>, CoreError> {
    let draft = store.draft(draft)?;
    Ok(draft
        .attachments
        .iter()
        .map(|attachment| AttachedFile {
            name: attachment.name.clone(),
            mime: attachment.mime.clone(),
            size: store
                .blobs()
                .size(attachment.blob)
                .or_log("an attachment's size could not be read"),
        })
        .collect())
}

/// Send this draft from a different account.
///
/// Both columns move together. `identity` is a foreign key into the *new* account's identities,
/// so writing one without the other leaves a draft that `identity_of` cannot resolve and nothing
/// can send — and the failure would arrive at Send, long after the choice was made. The identity
/// is therefore looked up before anything is written, so an account that has none leaves the
/// draft exactly where it was. It goes as the account's default identity, signed with that
/// identity's default in place of the signature it had ([`crate::signature::send_as`]).
pub fn move_draft_to(
    store: &SqliteStore,
    draft: DraftId,
    account: AccountId,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let draft = store.draft(draft)?;
    if draft.account == account {
        return Ok(draft);
    }
    crate::signature::send_as(store, draft.id, account, None, now)
}

/// A message just started with [`new_sealed_message`]: the draft, and what would stop it going.
#[derive(Debug)]
pub struct Composed {
    pub draft: Draft,
    /// Why it cannot be sent the way it asks, when it cannot: no key of the sender's own, a
    /// recipient with no key, a blind recipient on an encrypted message. Known straight away
    /// rather than found at Send; the draft is kept either way, as it is the user's to fix.
    pub refused: Option<CoreError>,
}

/// Start a new message to `[to, cc, bcc]`.
///
/// `receipt` is whether it asks its recipients for a read receipt.
pub fn new_message(
    store: &SqliteStore,
    from: Option<&str>,
    recipients: [&[Address]; 3],
    subject: &str,
    body: &str,
    receipt: ReceiptRequest,
    now: DateTime<Utc>,
) -> Result<Composed, CoreError> {
    new_sealed_message(
        store,
        from,
        recipients,
        subject,
        body,
        (receipt, None, None),
        now,
    )
}

/// [`new_message`], asking OpenPGP or S/MIME to sign, encrypt, or both when it is sent.
///
/// Says straight away what would stop an OpenPGP or S/MIME send, in [`Composed::refused`],
/// rather than leaving it to be found at Send.
pub fn new_sealed_message(
    store: &SqliteStore,
    from: Option<&str>,
    [to, cc, bcc]: [&[Address]; 3],
    subject: &str,
    body: &str,
    (receipt, openpgp, smime): (ReceiptRequest, Option<Mode>, Option<Mode>),
    now: DateTime<Utc>,
) -> Result<Composed, CoreError> {
    let account = account_for(store, from)?;
    let mut draft = draft_new(store, account, to, subject, body, now)?;
    if !cc.is_empty()
        || !bcc.is_empty()
        || receipt != draft.receipt
        || openpgp != draft.openpgp
        || smime != draft.smime
    {
        draft.cc = cc.to_vec();
        draft.bcc = bcc.to_vec();
        draft.receipt = receipt;
        draft.openpgp = openpgp;
        draft.smime = smime;
        save(store, &draft)?;
    }
    let mut refused = None;
    if draft.openpgp.is_some() || draft.smime.is_some() {
        let identity = identity_of(store, draft.account.clone(), Some(draft.identity))?;
        refused = crate::protect::check_both(store, &draft, &identity, now).err();
    }
    Ok(Composed { draft, refused })
}

/// The forward body: what the user wrote, then the original beneath a header block.
///
/// Not `>`-quoted. A forward is the message itself being passed on rather than answered, and
/// every client in the world writes it this way — headers first, then the text as it was — so a
/// recipient can see who sent it and when without taking our word for it.
fn forwarded<Tz: chrono::TimeZone>(body: &str, original: &Message, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body.trim_end());
        out.push_str("\r\n");
    }
    out.push_str(&format!("\r\n{FORWARDED}\r\n"));
    out.push_str(&format!(
        "From: {}\r\n",
        addresses(std::slice::from_ref(&original.from))
    ));
    out.push_str(&format!(
        "Date: {}\r\n",
        // Twenty-four hours, as the attribution line in [`quoted`] is written.
        crate::when::stamp(
            original.date,
            zone,
            crate::when::Stamp::Quote,
            crate::when::Clock::H24
        )
    ));
    out.push_str(&format!("Subject: {}\r\n", original.subject));
    // `To` and `Cc` as they were. Omitted when empty rather than written as a blank header,
    // which is what a header block copied from a message with no recipients would otherwise say.
    if !original.to.is_empty() {
        out.push_str(&format!("To: {}\r\n", addresses(&original.to)));
    }
    if !original.cc.is_empty() {
        out.push_str(&format!("Cc: {}\r\n", addresses(&original.cc)));
    }
    out.push_str("\r\n");
    if let Body::Present {
        text: Some(text), ..
    } = &original.body
    {
        for line in text.lines() {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    out
}

/// Forward a message, carrying it `carry`-wise, and return the draft.
pub fn forward(
    store: &SqliteStore,
    message: MessageId,
    to: &[Address],
    body: &str,
    carry: Carry,
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    match carry {
        Carry::Inline => draft_forward(store, message, to, body, now),
        Carry::Attached => draft_forward_attached(store, message, to, body, now),
    }
}

/// Write a draft back to the store.
///
/// Used by the composer on every save. `INSERT OR REPLACE` underneath, so this is also what an
/// autosave calls: the draft row is the document, and the widgets are only a view of it.
pub fn save(store: &SqliteStore, draft: &Draft) -> Result<(), CoreError> {
    store.apply(
        draft.account.clone(),
        &Patch {
            id: ChangeId::generate(),
            changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
        },
    )?;
    Ok(())
}

/// The plain drafts and templates an older build stored, their text written as flowed text with
/// every line hard, so a line ending in a space is not joined to the next once it goes out
/// labelled `format=flowed`. Migration 0038 queues them; each is rewritten once. Returns how
/// many.
pub fn reflow_stored(store: &SqliteStore) -> Result<usize, CoreError> {
    let queued = store.reflow_queue()?;
    for (kept, text) in &queued {
        store.reflowed(*kept, &plain_to_flowed(text))?;
    }
    Ok(queued.len())
}

/// Delete a draft.
///
/// There was no way to do this anywhere in the application. The composer's "Discard" only closed
/// the composer without saving — which for a draft that had never been saved is the same thing, and
/// for every other draft is not: a reply is persisted the moment it is created, and one opened
/// from the drafts list came off disk, so "Discard" left it sitting in Drafts for ever. Drafts
/// could be made and never unmade.
///
/// Returns the subject, so the caller can say what went.
pub fn discard(store: &SqliteStore, draft: DraftId) -> Result<String, CoreError> {
    let draft = store.draft(draft)?;
    // Mid-flight. Deleting the row would leave the outbox draining something that is no longer
    // there — and `Sending` in particular may already be on the wire, where nothing here can
    // recall it.
    if on_its_way(&draft.state) {
        return Err(CoreError::DraftQueued);
    }
    store.apply(
        draft.account.clone(),
        &Patch {
            id: ChangeId::generate(),
            changes: vec![Change::DraftDelete(draft.id)],
        },
    )?;
    // And off the server, where its copy was kept ([`keep_on_server`]). Discarded whatever
    // becomes of that: the draft is gone here, and a copy left there is only a copy.
    if let Some(mailbox) = drafts_on_server(store, &draft.account) {
        mail_runtime::server_drafts::queue_drop(store, &draft, mailbox, Utc::now())
            .or_log("the discarded draft's copy on the server was not queued to go");
    }
    Ok(draft.subject)
}

/// Keep the draft's latest in the server's Drafts mailbox, in place of the copy kept there
/// before, as Apple Mail and Thunderbird do: so a draft begun here can be finished on another
/// device. Queued, like everything sent to a server ([`mail_runtime::server_drafts`]); an upload
/// of it still waiting is replaced. Whether anything was queued.
///
/// Nothing, and no error, for a draft with nowhere to go: a POP3 or local account, or a server
/// that named no Drafts folder, where uploading to a guessed path is how a copy lands somewhere
/// nobody looks. Nothing for a Microsoft Graph account either, which makes a message from MIME
/// only as a new draft with an id of its own, so there is no copy to replace; such a draft is
/// this client's alone, as every draft was. Nothing for a draft on its way out, whose copy goes
/// when it is sent. And nothing for a draft that asks to be encrypted, which is not put on a
/// server in the clear; a copy kept before it asked goes when it is sent or discarded.
///
/// The copy names every addressee, `Bcc` included, as only the sender reads it, and carries the
/// draft's own `Message-ID`, which is how the next upload finds it.
pub fn keep_on_server(
    store: &SqliteStore,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<bool, CoreError> {
    let draft = store.draft(draft)?;
    if draft.state != SendState::Editing {
        return Ok(false);
    }
    let Some(mailbox) = drafts_on_server(store, &draft.account) else {
        return Ok(false);
    };
    if draft.openpgp.is_some_and(Mode::encrypts) || draft.smime.is_some_and(Mode::encrypts) {
        // Asked to be encrypted after a copy was kept: the copy in the clear goes, and the
        // upload still waiting with it, which would have put the plaintext there.
        mail_runtime::server_drafts::queue_drop(store, &draft, mailbox, now)?;
        return Ok(false);
    }
    Ok(mail_runtime::server_drafts::queue_keep(
        store, &draft, mailbox, now,
    )?)
}

/// The Drafts mailbox `account` keeps copies of drafts in on its server: the one its server
/// named, on an IMAP or JMAP account that is not Gmail's
/// ([`mail_runtime::server_drafts::keeps_copies`]). See [`keep_on_server`] for why no other.
fn drafts_on_server(store: &SqliteStore, account: &AccountId) -> Option<MailboxRef> {
    let stored = store
        .account(account.clone())
        .or_log("the account a draft is on could not be read")??;
    let plan = stored
        .plan
        .or_log("the plan of the account a draft is on could not be read")?;
    if !plan.incoming.caps().server_drafts {
        return None;
    }
    let caps = stored
        .caps?
        .or_log("the capabilities of the account a draft is on could not be read")?;
    if !mail_runtime::server_drafts::keeps_copies(&caps) {
        return None;
    }
    let path = caps.folders.path(MailboxRole::Drafts)?;
    Some(MailboxRef {
        account: account.clone(),
        path: path.to_owned(),
    })
}

/// A new message's text and HTML: `body`, then the identity's default signature for new mail.
///
/// HTML only when the signature was written in the composer and so has some: the document is
/// the body's paragraphs, the separator and the signature's own, as the composer would write it,
/// so the formatting goes out in the HTML part. A signature that is only text leaves the draft
/// text only, as it always was.
fn signed_new(
    store: &SqliteStore,
    identity: &Identity,
    body: &str,
) -> Result<(String, Option<String>), CoreError> {
    let signature = signature::default_for(store, identity.id, SignatureUse::NewMail)?;
    let text = signed(body, signature.as_ref());
    let html = signature
        .as_ref()
        .filter(|signature| signature.html.is_some())
        .map(|signature| {
            let mut nodes = mail_editor::nodes_from_plain(body.trim());
            if nodes.is_empty() {
                nodes.push(mail_editor::Node::plain(
                    mail_editor::ParaKind::Paragraph,
                    "",
                ));
            }
            nodes.push(mail_editor::Node::Object(mail_editor::Object::Signature));
            nodes.extend(signature::nodes_of(signature));
            mail_editor::to_html(&mail_editor::Doc { nodes })
        });
    Ok((text, html))
}

/// Put a signature beneath what was written.
///
/// Appended at compose time rather than at send, like the quoted material beside it, so the user
/// can see it, edit it, or take it off for one message. A signature the user cannot delete from
/// a particular reply is worse than none.
///
/// The delimiter is `"-- "` on its own line — two hyphens, a space, and nothing else. RFC 3676
/// §4.3 names that exact string, and every client that trims a signature when quoting looks for
/// it; `--` without the trailing space is a different line and gets quoted back at people for
/// the rest of the thread.
fn signed(body: &str, signature: Option<&Signature>) -> String {
    let Some(signature) = signature.map(|signature| signature.text.trim_end()) else {
        return body.to_owned();
    };
    if signature.is_empty() {
        return body.to_owned();
    }
    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body.trim_end());
        out.push_str("\r\n");
    }
    out.push_str("\r\n-- \r\n");
    for line in signature.lines() {
        out.push_str(line);
        out.push_str("\r\n");
    }
    out
}

/// What [`set_signature`] did, and to whose signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureChange {
    /// The address of the identity it was set on.
    pub email: String,
    /// Whether the identity now has a signature: `false` when it was cleared.
    pub set: bool,
}

/// Set or clear the signature on an account's default identity.
pub fn set_signature(
    store: &SqliteStore,
    account: AccountId,
    signature: Option<&str>,
) -> Result<SignatureChange, CoreError> {
    let identity = identity_of(store, account, None)?;
    // An empty string is not a signature: stored as NULL, so "has one" is a single question
    // rather than two that can disagree.
    let trimmed = signature.map(str::trim_end).filter(|s| !s.is_empty());
    store.set_signature(identity.id, trimmed)?;
    Ok(SignatureChange {
        email: identity.from.email,
        set: trimmed.is_some(),
    })
}

/// The reply body, with the original quoted beneath an attribution line.
///
/// Plain `>` quoting and nothing cleverer: this is the one format every mail client in the
/// world renders correctly, including the ones that predate HTML mail.
fn quoted<Tz: chrono::TimeZone>(body: &str, original: &Message, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    // The text part, when there is one. A message whose body was never fetched quotes nothing
    // rather than quoting the word "None".
    let text = match &original.body {
        Body::Present {
            text: Some(text), ..
        } => Some(text.as_str()),
        _ => None,
    };
    quoting(body, original, text, zone)
}

/// [`quoted`], quoting `text` beneath the attribution line rather than the whole of the
/// original's: what a reply to a selection quotes.
fn quoting<Tz: chrono::TimeZone>(
    body: &str,
    original: &Message,
    text: Option<&str>,
    zone: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let who = original
        .from
        .name
        .clone()
        .unwrap_or_else(|| original.from.email.clone());
    // In the sender's own zone. Quoted in UTC this said "at 01:02" for a message written at
    // 09:02, in a line the recipient reads and cannot correct.
    //
    // Twenty-four hours, whatever this person's clock: the line is read in someone else's
    // mailbox, by a reader whose clock is not known here, and it is the one reading nobody takes
    // for the other half of the day.
    let when = crate::when::stamp(
        original.date,
        zone,
        crate::when::Stamp::Quote,
        crate::when::Clock::H24,
    );
    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body.trim_end());
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.push_str(&format!("On {when}, {who} wrote:\r\n"));
    for line in text.into_iter().flat_map(str::lines) {
        out.push_str(&format!("> {line}\r\n"));
    }
    out
}

/// Whether a draft has a submission in the outbox that has not been settled: queued, held for
/// later, or on the wire. Its bytes are frozen, so editing or discarding it would be editing
/// something that is no longer what goes out.
fn on_its_way(state: &SendState) -> bool {
    matches!(
        state,
        SendState::Queued | SendState::Scheduled { .. } | SendState::Sending
    )
}

/// When a queued message may leave the outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaves {
    /// As soon as a pass drains the outbox.
    Now,
    /// Not before this instant. The draft shows [`SendState::Scheduled`] until it goes.
    At(DateTime<Utc>),
}

/// Queue a draft for delivery, returning it and what was queued.
///
/// Builds the bytes and the envelope together, freezes the bytes in the blob store, and puts a
/// submission in the outbox. Nothing here touches the network: the next sync delivers it, and
/// until it does the message is safe across a restart.
///
/// For a draft that is neither signed nor encrypted: a draft that asks to be has no keys here
/// and is refused (see [`send_with`], over the [`Mail::signing`](crate::Mail::signing) store,
/// for one that may be).
pub fn send(
    store: &SqliteStore,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<(Draft, mail_mime::Posting), CoreError> {
    Ok(send_with(
        store,
        &NoKeys,
        &crate::pgp::no_passphrase,
        draft,
        now,
    )?)
}

/// A store of signing keys with none in it, for the sends that carry none: a draft that asks to be
/// signed or encrypted is refused rather than looked up in a keyring this function was not given.
struct NoKeys;

impl mail_secrets::SigningStore for NoKeys {
    fn get(&self, _: &SigningKeyRef) -> Result<SigningSecret, CredentialError> {
        Err(Self::refusal())
    }

    fn put(&self, _: &SigningKeyRef, _: &SigningSecret) -> Result<(), CredentialError> {
        Err(Self::refusal())
    }

    fn forget(&self, _: &SigningKeyRef) -> Result<(), CredentialError> {
        Err(Self::refusal())
    }
}

impl NoKeys {
    fn refusal() -> CredentialError {
        CredentialError::Secrets(Failure::said("no signing keys are available to this send"))
    }
}

/// Why a send was refused, typed so a caller can act on it: an OpenPGP key that needs its
/// passphrase ([`SendError::locked`]) is asked for and the send tried again. Its text is what the
/// command line has always printed.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("{0}")]
    Pgp(#[from] crate::pgp::PgpError),
    #[error("{0}")]
    Smime(#[from] crate::smime::SmimeError),
    /// Anything else: the draft already sent, no server to send from, the store.
    #[error("{0}")]
    Other(CoreError),
}

/// The two protections keep their own variants, because [`SendError::locked`] looks for one.
impl From<CoreError> for SendError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Pgp(e) => SendError::Pgp(e),
            CoreError::Smime(e) => SendError::Smime(e),
            other => SendError::Other(other),
        }
    }
}

impl From<SendError> for CoreError {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Pgp(e) => CoreError::Pgp(e),
            SendError::Smime(e) => CoreError::Smime(e),
            SendError::Other(other) => other,
        }
    }
}

impl From<TimeError> for SendError {
    fn from(error: TimeError) -> Self {
        CoreError::from(error).into()
    }
}

// The window's older callers keep a `String` error; the text is the same either way.
impl From<SendError> for String {
    fn from(error: SendError) -> Self {
        error.to_string()
    }
}

impl SendError {
    /// The OpenPGP key whose passphrase was not given, or was wrong, when that is why.
    pub fn locked(&self) -> Option<Fingerprint> {
        match self {
            SendError::Pgp(crate::pgp::PgpError::Locked(fingerprint)) => Some(*fingerprint),
            _ => None,
        }
    }
}

/// [`send`], with the keyring an OpenPGP draft's secret key is read from and whoever is asked
/// for its passphrase named.
pub fn send_with(
    store: &SqliteStore,
    secrets: &dyn mail_secrets::SigningStore,
    ask: crate::pgp::Ask<'_>,
    draft: DraftId,
    now: DateTime<Utc>,
) -> Result<(Draft, mail_mime::Posting), SendError> {
    queue_with(store, secrets, ask, draft, Leaves::Now, now)
}

/// A send held for later: what was queued, and when it may leave.
#[derive(Debug)]
pub struct Scheduled {
    pub draft: Draft,
    pub post: mail_mime::Posting,
    pub at: DateTime<Utc>,
}

/// Queue a draft to leave at `phrase`, in the words [`crate::snooze::snooze_until`] takes:
/// `tomorrow`, `tonight`, `+2h`, `2026-09-25 09:00`.
pub fn send_later(
    store: &SqliteStore,
    draft: DraftId,
    phrase: &str,
    now: DateTime<Utc>,
) -> Result<Scheduled, CoreError> {
    send_later_in(store, draft, phrase, now, &Local)
}

/// [`send_later`], with the keyring and the passphrase prompt named, as [`send_with`] has them.
pub fn send_later_with(
    store: &SqliteStore,
    secrets: &dyn mail_secrets::SigningStore,
    ask: crate::pgp::Ask<'_>,
    draft: DraftId,
    phrase: &str,
    now: DateTime<Utc>,
) -> Result<Scheduled, SendError> {
    let at = crate::snooze::snooze_until(phrase, now, &Local)?;
    let (draft, post) = queue_with(store, secrets, ask, draft, Leaves::At(at), now)?;
    Ok(Scheduled { draft, post, at })
}

/// The same, with the zone `phrase` is read in named.
pub fn send_later_in<Tz: chrono::TimeZone>(
    store: &SqliteStore,
    draft: DraftId,
    phrase: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Scheduled, CoreError>
where
    Tz::Offset: std::fmt::Display,
{
    let at = crate::snooze::snooze_until(phrase, now, zone)?;
    let (draft, post) = queue(store, draft, Leaves::At(at), now)?;
    Ok(Scheduled { draft, post, at })
}

/// Freeze a draft's bytes and put its submission in the outbox, to leave as `leaves` says.
///
/// For a draft that is neither signed nor encrypted, as [`send`] is. Returns the draft and what
/// was queued. A draft that is already queued, held or failing is
/// taken back first and queued afresh, so pressing Send twice — or changing the time of a
/// scheduled one — leaves one submission in the outbox rather than two, which the recipient
/// would have received twice.
pub fn queue(
    store: &SqliteStore,
    draft: DraftId,
    leaves: Leaves,
    now: DateTime<Utc>,
) -> Result<(Draft, mail_mime::Posting), CoreError> {
    queue_inner(
        store,
        &NoKeys,
        &crate::pgp::no_passphrase,
        (draft, PictureSize::Actual),
        leaves,
        now,
    )
}

/// [`queue`], with the keyring and the passphrase prompt named.
///
/// The keyring is read only for a draft that asks to be signed or encrypted; `ask` is asked only
/// when that key is passphrase-protected. The OpenPGP work happens here, as the bytes are
/// frozen, so what the outbox holds is already signed and encrypted — see `protect::Seal`.
pub fn queue_with(
    store: &SqliteStore,
    secrets: &dyn mail_secrets::SigningStore,
    ask: crate::pgp::Ask<'_>,
    draft: DraftId,
    leaves: Leaves,
    now: DateTime<Utc>,
) -> Result<(Draft, mail_mime::Posting), SendError> {
    queue_sized(store, secrets, ask, draft, leaves, PictureSize::Actual, now)
}

/// [`queue_with`], its pictures sent at `pictures`: each JPEG or PNG it carries made smaller as
/// the bytes are frozen (`pictures::sized`). The draft keeps the originals, so the choice can be
/// made again; only what goes is smaller. Decodes every picture, so it is for a thread that does
/// not draw, unless `pictures` is [`PictureSize::Actual`].
pub fn queue_sized(
    store: &SqliteStore,
    secrets: &dyn mail_secrets::SigningStore,
    ask: crate::pgp::Ask<'_>,
    draft: DraftId,
    leaves: Leaves,
    pictures: PictureSize,
    now: DateTime<Utc>,
) -> Result<(Draft, mail_mime::Posting), SendError> {
    queue_inner(store, secrets, ask, (draft, pictures), leaves, now).map_err(SendError::from)
}

fn queue_inner(
    store: &SqliteStore,
    secrets: &dyn mail_secrets::SigningStore,
    ask: crate::pgp::Ask<'_>,
    (draft, pictures): (DraftId, PictureSize),
    leaves: Leaves,
    now: DateTime<Utc>,
) -> Result<(Draft, mail_mime::Posting), CoreError> {
    // Before anything is taken back, so a time that has gone leaves an existing schedule alone.
    if let Leaves::At(at) = leaves
        && at <= now
    {
        return Err(CoreError::SendTimePassed {
            // The command line's refusal (it names `--at`); the window refuses a time that
            // has passed before it gets here, in its own words.
            stamp: crate::when::stamp(
                at,
                &Local,
                crate::when::Stamp::Full,
                crate::when::Clock::H24,
            ),
        });
    }
    let mut draft = store.draft(draft)?;
    match draft.state {
        SendState::Sent { at, .. } => {
            return Err(CoreError::DraftSentAt { at });
        }
        SendState::Sending => return Err(CoreError::AlreadySending),
        SendState::Queued | SendState::Scheduled { .. } | SendState::Failed { .. } => {
            draft = unsend(store, draft.id, now)?;
        }
        SendState::Editing => {}
    }
    // Refused before anything is built or queued: an entry in the outbox of an account with no
    // server would sit there for ever, and the draft would say "queued" the whole time.
    if crate::sync::local_accounts(store).contains(&draft.account) {
        return Err(mail_runtime::RuntimeError::NoServer("send from").into());
    }
    let identity = identity_of(store, draft.account.clone(), Some(draft.identity))?;
    let parent = draft
        .in_reply_to
        .and_then(|id| store.message(id).or_log("a message could not be read"));

    // Attachment bytes come from the blob store, because `posting` is pure and takes them as
    // an argument rather than reading anything.
    let mut parts = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        let bytes = store
            .blobs()
            .get(attachment.blob)
            .map_err(|e| CoreError::context(format!("attachment {}", attachment.name), e))?;
        let bytes = match pictures::sized(&bytes, pictures) {
            std::borrow::Cow::Owned(smaller) => smaller,
            std::borrow::Cow::Borrowed(_) => bytes,
        };
        parts.push((attachment.blob, bytes));
    }

    let mut post = posting(&draft, &identity, parent.as_ref(), &parts)?;
    // Before anything is taken back or queued, so a send OpenPGP or S/MIME refuses (no key for a
    // recipient, no passphrase, both asked at once) leaves the draft as it was.
    crate::protect::check_both(store, &draft, &identity, now)?;
    post.message = OpenPgp::outgoing(store, secrets, ask, &draft, &identity, post.message, now)?;
    post.message = Smime::outgoing(store, secrets, ask, &draft, &identity, post.message, now)?;
    let raw = store.blobs().put(&post.message)?;

    let queued = store.enqueue(
        draft.account.clone(),
        RemoteIntent::Send {
            draft: draft.id,
            raw,
            mail_from: post.mail_from.clone(),
            rcpt_to: post.rcpt_to.clone(),
            delivery: post.delivery,
        },
        // No undo. Unsending is not something the outbox can offer once the bytes are on
        // the wire, and a patch that pretended otherwise would revert something real.
        &Patch {
            id: ChangeId::generate(),
            changes: Vec::new(),
        },
        // The entry's `next_attempt`, which is the whole of what holds a scheduled send:
        // the outbox does not hand out an entry before it.
        match leaves {
            Leaves::Now => now,
            Leaves::At(at) => at,
        },
    )?;
    if queued.is_none() {
        return Err(CoreError::SubmissionNotQueued);
    }
    let state = match leaves {
        Leaves::Now => SendState::Queued,
        Leaves::At(at) => SendState::Scheduled { at },
    };
    store.set_send_state(draft.id, &state, now)?;
    Ok((
        Draft {
            state,
            updated: now,
            ..draft
        },
        post,
    ))
}

/// Take back a send that has not left: the queued submission goes, the draft is editable again.
///
/// One write of two changes. Deleting the draft withdraws its outbox entry (334b758), and the
/// upsert puts the same draft back as `Editing`, so what comes back is exactly what was sent.
/// A draft already `Sending` is on the wire and cannot be recalled; `Sent` is history.
pub fn unsend(store: &SqliteStore, draft: DraftId, now: DateTime<Utc>) -> Result<Draft, CoreError> {
    let stored = store.draft(draft)?;
    match stored.state {
        SendState::Sending => return Err(CoreError::AlreadySending),
        SendState::Sent { .. } => return Err(CoreError::AlreadySent),
        SendState::Editing
        | SendState::Queued
        | SendState::Scheduled { .. }
        | SendState::Failed { .. } => {}
    }
    let back = Draft {
        state: SendState::Editing,
        updated: now,
        ..stored
    };
    store.apply(
        back.account.clone(),
        &Patch {
            id: ChangeId::generate(),
            changes: vec![
                Change::DraftDelete(back.id),
                Change::DraftUpsert(Box::new(back.clone())),
            ],
        },
    )?;
    // A reminder was for this send, and this send is not happening.
    crate::follow_up::release(store, back.id)?;
    Ok(back)
}

/// Every draft of every account, oldest account first, each with the state it got to.
pub fn drafts(store: &SqliteStore) -> Result<Vec<Draft>, CoreError> {
    let mut all = Vec::new();
    for account in store.list_accounts()? {
        all.extend(store.drafts(account.id)?);
    }
    Ok(all)
}

fn addresses(list: &[Address]) -> String {
    Address::join(list)
}

#[cfg(test)]
mod carried_tests {
    use super::*;

    fn part(name: &str, size: u64) -> Attachment {
        Attachment {
            name: name.to_owned(),
            mime: "application/octet-stream".to_owned(),
            size,
            content: PartContent::Held(BlobId::generate()),
            inline: Inline::Attached,
        }
    }

    /// A part too large for what is left of the budget is left behind and takes none of it: a
    /// smaller one after it still goes.
    #[test]
    fn a_part_left_behind_does_not_count_against_the_ones_after_it() {
        let parts = [
            part("small.bin", ATTACHMENT_BUDGET / 4),
            part("huge.bin", ATTACHMENT_BUDGET),
            part("last.bin", ATTACHMENT_BUDGET / 4),
        ];
        let kept: Vec<String> = carried(&parts).into_iter().map(|p| p.name).collect();
        assert_eq!(kept, ["small.bin", "last.bin"]);
    }
}

#[cfg(test)]
mod server_copy_tests {
    use super::*;
    use crate::support_tests::{account, at, caps};

    /// An account on `incoming` whose server lists the folders `roles`, with a draft begun on it.
    fn writing(incoming: Incoming, roles: Vec<(String, MailboxRole)>) -> (SqliteStore, Draft) {
        let store = mail_store::testing::in_memory();
        let plan = AccountPlan {
            address: "me@example.test".to_owned(),
            incoming,
            outgoing: Outgoing::Smtp {
                host: "smtp.example.test".to_owned(),
                port: 465,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
            identities: Vec::new(),
        };
        mail_store::testing::seed_account_plan(&store, account(), "me@example.test", &plan, None);
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            account(),
            "me@example.test",
            None,
        );
        let caps = AccountCaps {
            folders: FolderRoles(roles),
            ..caps()
        };
        mail_store::testing::seed_caps(&store, account(), &caps, at(0)).unwrap();
        let to = [Address {
            name: None,
            email: "bea@example.test".to_owned(),
        }];
        let draft = draft_new(&store, account(), &to, "half a thought", "", at(1)).unwrap();
        (store, draft)
    }

    fn imap() -> Incoming {
        Incoming::Imap {
            host: "imap.example.test".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        }
    }

    fn with_drafts() -> Vec<(String, MailboxRole)> {
        vec![("Drafts".to_owned(), MailboxRole::Drafts)]
    }

    /// Everything queued, whenever it is due: a discard queues at the clock's now.
    fn queued(store: &SqliteStore) -> Vec<ProtoOp> {
        store
            .outbox_due(account(), Utc::now() + chrono::TimeDelta::days(1))
            .unwrap()
            .into_iter()
            .map(|entry| entry.op)
            .collect()
    }

    #[test]
    fn a_draft_on_an_imap_account_with_a_drafts_folder_is_queued_for_it_by_its_message_id() {
        let (store, draft) = writing(imap(), with_drafts());
        assert!(keep_on_server(&store, draft.id, at(2)).unwrap());
        let ops = queued(&store);
        let [
            ProtoOp::KeepDraft {
                draft: kept,
                mailbox,
                message_id,
            },
        ] = &ops[..]
        else {
            panic!("{ops:?}");
        };
        assert_eq!(*kept, draft.id);
        assert_eq!(mailbox.path, "Drafts");
        assert_eq!(*message_id, format!("{}@example.test", draft.id));
        // Nothing is stored for the upload: its bytes are made from the draft when it runs.
        let bytes = String::from_utf8(
            mail_runtime::server_drafts::copy(&store, draft.id)
                .unwrap()
                .expect("a draft being written has a copy"),
        )
        .unwrap();
        assert!(
            bytes.contains(&format!("Message-ID: <{message_id}>\r\n")),
            "{bytes}"
        );

        // A second copy takes the first one's place in the outbox.
        assert!(keep_on_server(&store, draft.id, at(3)).unwrap());
        assert_eq!(queued(&store).len(), 1, "{:?}", queued(&store));
    }

    #[test]
    fn discarding_the_draft_queues_its_copy_s_removal_in_place_of_the_upload() {
        let (store, draft) = writing(imap(), with_drafts());
        keep_on_server(&store, draft.id, at(2)).unwrap();
        discard(&store, draft.id).unwrap();
        let ops = queued(&store);
        assert!(
            matches!(&ops[..], [ProtoOp::DropDraft { draft: gone, .. }] if *gone == draft.id),
            "{ops:?}"
        );
    }

    #[test]
    fn nothing_is_queued_where_there_is_no_drafts_folder_to_keep_a_copy_in() {
        let pop3 = Incoming::Pop3 {
            host: "pop.example.test".to_owned(),
            port: 995,
            tls: Tls::Implicit,
            leave: LeaveOnServer::Keep,
        };
        // (incoming, the folders its server lists, why nothing goes)
        let cases = [
            (imap(), Vec::new(), "the server named no Drafts folder"),
            (pop3, with_drafts(), "a maildrop has no folders"),
            (
                Incoming::Graph,
                with_drafts(),
                "Graph keeps no copy to replace",
            ),
        ];
        for (incoming, roles, why) in cases {
            let (store, draft) = writing(incoming, roles);
            assert!(!keep_on_server(&store, draft.id, at(2)).unwrap(), "{why}");
            discard(&store, draft.id).unwrap();
            assert!(queued(&store).is_empty(), "{why}: {:?}", queued(&store));
        }
    }

    #[test]
    fn a_draft_that_asks_to_be_encrypted_is_not_put_on_the_server_in_the_clear() {
        let (store, draft) = writing(imap(), with_drafts());
        for (openpgp, smime) in [
            (Some(Mode::Encrypt), None),
            (None, Some(Mode::SignAndEncrypt)),
        ] {
            save(
                &store,
                &Draft {
                    openpgp,
                    smime,
                    ..draft.clone()
                },
            )
            .unwrap();
            assert!(!keep_on_server(&store, draft.id, at(2)).unwrap());
            // Only a removal may be queued, for a copy an earlier session may have kept in the
            // clear; never an upload.
            let ops = queued(&store);
            assert!(
                ops.iter().all(
                    |op| matches!(op, ProtoOp::DropDraft { draft: gone, .. } if *gone == draft.id)
                ),
                "{ops:?}"
            );
        }
    }

    #[test]
    fn a_draft_that_asks_to_be_encrypted_takes_the_copy_kept_before_and_its_waiting_upload_away() {
        let (store, draft) = writing(imap(), with_drafts());
        assert!(keep_on_server(&store, draft.id, at(2)).unwrap());
        save(
            &store,
            &Draft {
                openpgp: Some(Mode::Encrypt),
                ..draft.clone()
            },
        )
        .unwrap();
        assert!(!keep_on_server(&store, draft.id, at(3)).unwrap());
        let ops = queued(&store);
        assert!(
            matches!(&ops[..], [ProtoOp::DropDraft { draft: gone, .. }] if *gone == draft.id),
            "{ops:?}"
        );
    }

    #[test]
    fn gmail_keeps_no_copy_of_a_draft_on_its_server() {
        let (store, draft) = writing(imap(), with_drafts());
        let gmail = AccountCaps {
            archive: mail_domain::ArchiveMeans::DropInbox,
            folders: FolderRoles(with_drafts()),
            ..caps()
        };
        mail_store::testing::seed_caps(&store, account(), &gmail, at(0)).unwrap();
        assert!(!keep_on_server(&store, draft.id, at(2)).unwrap());
        assert!(queued(&store).is_empty(), "{:?}", queued(&store));
    }
}

#[cfg(test)]
mod reflow_tests {
    use super::*;
    use crate::support_tests::{account, at};

    /// A plain draft an older build kept, with its text as it was written, and queued as
    /// migration 0038 queues it.
    fn kept_plain(store: &SqliteStore, text: &str) -> DraftId {
        let identity = Identity {
            id: IdentityId::generate(),
            account: account(),
            from: Address {
                name: None,
                email: "me@example.test".to_owned(),
            },
            reply_to: None,
            signature: None,
            default: IsDefault::Default,
        };
        mail_store::testing::seed_account(store, account(), "me@example.test");
        mail_store::testing::seed_identity(store, &identity);
        let draft = Draft::blank(&identity, at(1));
        save(store, &draft).unwrap();
        mail_store::testing::keep_unflowed_plain_draft(store, draft.id, text);
        draft.id
    }

    /// Sent as `format=flowed` as it was kept, a line ending in a space would join the next
    /// and a leading space would be taken for stuffing.
    #[test]
    fn a_plain_draft_kept_before_flowed_text_is_rewritten_once() {
        let store = mail_store::testing::in_memory();
        let id = kept_plain(
            &store,
            "Dear Ada, \nthe numbers:\n  12\nFrom here on\n> quoted \n-- \nBea",
        );
        assert_eq!(reflow_stored(&store).unwrap(), 1);
        let text = store.draft(id).unwrap().text;
        assert_eq!(
            text,
            "Dear Ada,\nthe numbers:\n   12\n From here on\n> quoted\n-- \nBea"
        );
        assert_eq!(
            mail_mime::unflow(&text, mail_mime::Delsp::No),
            "Dear Ada,\nthe numbers:\n  12\nFrom here on\n> quoted\n-- \nBea",
            "and reads back as it was written"
        );
        assert_eq!(reflow_stored(&store).unwrap(), 0, "once");
        assert_eq!(store.draft(id).unwrap().text, text);
    }
}
