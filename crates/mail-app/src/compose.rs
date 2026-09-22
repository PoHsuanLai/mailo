//! Composing from the command line: reply, send, and what is waiting.
//!
//! Split from [`crate::cli`] because these three are the only commands that *write* mail, and
//! because the interesting part is not the parsing — it is that "send" means "freeze the bytes
//! and queue them", never "open a connection". A send that depended on the network being up at
//! the moment the user pressed the key would lose the message on a train.

use chrono::{DateTime, Local, Utc};
use mail_domain::*;
use mail_mime::posting;
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;

/// Read one identity, or the account's default, out of the `identities` table.
///
/// The table rather than `AccountPlan.identities`, and that choice matters: a draft's
/// `identity` column is a foreign key into this table, so any draft that exists at all has a
/// row here. The plan's copy is written at account creation and can go stale — an account added
/// before identities were created at setup has neither, but one restored from a plan alone
/// would have a list the table does not back. Reading the side the constraint enforces means
/// the lookup cannot disagree with the row that let the draft be saved.
fn identity_of(
    store: &SqliteStore,
    account: AccountId,
    wanted: Option<IdentityId>,
) -> Result<Identity, String> {
    let db = store.connection();
    // `is_default` sorts 'alternate' before 'default', so DESC puts the default first; the id
    // breaks the tie so an account with two alternates picks the same one every time.
    let (sql, param): (&str, String) = match wanted {
        Some(id) => (
            "SELECT id, from_name, from_email, reply_to, signature, is_default
             FROM identities WHERE id = ?1",
            id.to_string(),
        ),
        None => (
            "SELECT id, from_name, from_email, reply_to, signature, is_default
             FROM identities WHERE account = ?1 ORDER BY is_default DESC, id LIMIT 1",
            account.to_string(),
        ),
    };
    let mut stmt = db.prepare(sql).map_err(|e| e.to_string())?;
    let found = stmt
        .query_row([param], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(|_| match wanted {
            Some(id) => format!("the draft names identity {id}, which this account no longer has"),
            None => "this account has no identity to send as. It was added before identities \
                     were created at setup; re-add it with: mailo account add <address>"
                .to_owned(),
        })?;

    let (id, from_name, from_email, reply_to, signature, is_default) = found;
    let parse = |what: &str, text: &str| -> Result<serde_json::Value, String> {
        serde_json::from_str(text).map_err(|e| format!("stored {what} is unreadable: {e}"))
    };
    Ok(Identity {
        id: IdentityId::from_uuid(
            id.parse()
                .map_err(|_| format!("stored identity id {id:?} is unreadable"))?,
        ),
        account,
        from: Address {
            name: from_name,
            email: from_email,
        },
        reply_to: match reply_to {
            Some(text) => serde_json::from_str(&text)
                .map_err(|e| format!("stored reply_to is unreadable: {e}"))?,
            None => None,
        },
        signature,
        default: serde_json::from_value(parse("is_default", &is_default)?)
            .map_err(|e| format!("stored is_default is unreadable: {e}"))?,
    })
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
) -> Result<Draft, String> {
    draft_reply_in(store, message, scope, body, now, &Local)
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
) -> Result<Draft, String>
where
    Tz::Offset: std::fmt::Display,
{
    let original = store.message(message).map_err(|e| e.to_string())?;
    let identity = identity_of(store, original.account, None)?;

    let mut draft = Draft::reply_to(&original, &identity, scope, now);
    // `Draft::reply_to` leaves the text empty on purpose — quoting is a rendering decision, not
    // a property of the draft — so the quoting happens here, where the renderer is.
    draft.text = quoted(&signed(body, &identity), &original, zone);
    save(store, &draft)?;
    Ok(draft)
}

/// Create and persist a forward of `message`, addressed to `to`.
///
/// Forwarding was modelled and unreachable: `Draft::forward_of` has existed and been tested in
/// `mail-domain` since phase 1, and no surface in the application called it. A mail client that
/// cannot forward is not one, and the shape was already there — what was missing is the half
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
) -> Result<Draft, String> {
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
) -> Result<Draft, String>
where
    Tz::Offset: std::fmt::Display,
{
    let original = store.message(message).map_err(|e| e.to_string())?;
    let identity = identity_of(store, original.account, None)?;

    let mut draft = Draft::forward_of(&original, &identity, now);
    draft.to = to.to_vec();
    draft.text = forwarded(&signed(body, &identity), &original, zone);
    save(store, &draft)?;
    Ok(draft)
}

/// Every account that can send, as `(address, id)`, oldest first.
///
/// Oldest first rather than alphabetical so the list does not reorder itself when an account is
/// added — a picker whose first entry moves is a picker that sends from the wrong address.
pub fn sending_accounts(store: &SqliteStore) -> Vec<(String, AccountId)> {
    let db = store.connection();
    let Ok(mut stmt) = db.prepare("SELECT address, id FROM accounts ORDER BY created_at") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
    else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok())
        .filter_map(|(address, id)| Some((address, AccountId::from_uuid(id.parse().ok()?))))
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
pub fn account_for(store: &SqliteStore, wanted: Option<&str>) -> Result<AccountId, String> {
    let accounts = sending_accounts(store);
    if accounts.is_empty() {
        return Err("no accounts. Add one with: mailo account add <address>".to_owned());
    }
    match wanted {
        Some(address) => accounts
            .iter()
            .find(|(had, _)| had.eq_ignore_ascii_case(address))
            .map(|(_, id)| *id)
            .ok_or_else(|| {
                let known: Vec<&str> = accounts.iter().map(|(a, _)| a.as_str()).collect();
                format!("no account {address}. This one has: {}", known.join(", "))
            }),
        None if accounts.len() == 1 => Ok(accounts[0].1),
        None => {
            let known: Vec<&str> = accounts.iter().map(|(a, _)| a.as_str()).collect();
            Err(format!(
                "which account should this be sent from? Say --from <address>: {}",
                known.join(", ")
            ))
        }
    }
}

/// Create and persist a message that answers nothing, returning the draft.
///
/// The gap phase 7 opened with: `Draft` has carried every field a new message needs since phase
/// 1, `Composer` has edited one since phase 6 and `send` has sent one since phase 4, and there
/// was no way to *make* one that was not a reply or a forward. Mail could only be written to
/// someone who had written first.
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
) -> Result<Draft, String> {
    let identity = identity_of(store, account, None)?;
    let mut draft = Draft::blank(&identity, now);
    draft.to = to.to_vec();
    draft.subject = subject.to_owned();
    // The signature, and nothing else. There is no original to quote and no header block to
    // write, which is the whole difference between this and the other two.
    draft.text = signed(body, &identity);
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
        _ => "application/octet-stream",
    }
}

/// Attach `bytes` to a draft under `name`.
///
/// Bytes rather than a path, because the window has a file chooser that hands over contents and
/// the command line has a path, and only one of those is I/O this function should be doing.
///
/// `PendingAttachment` has existed since phase 1, `mail_mime::build` has assembled multipart
/// from it since phase 2 and `sqlite/draft.rs` has persisted it since phase 3 — and nothing in
/// the application ever constructed one, so the send path supported attachments right up to the
/// moment somebody needed one. See FINDINGS F138.
pub fn attach_bytes(
    store: &SqliteStore,
    draft: DraftId,
    name: &str,
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let mut draft = store.draft(draft).map_err(|e| e.to_string())?;
    if matches!(draft.state, SendState::Queued | SendState::Sending) {
        return Err(
            "that draft is on its way; what goes out was frozen when you sent it".to_owned(),
        );
    }
    // The name a file arrives under is the name it goes out under, and it is about to be a
    // header. `safe_name` is the same function that decides where an *incoming* attachment may
    // be written, so a `../` or a newline is refused in both directions by one rule.
    let name = crate::attach::safe_name(name);

    let carried: u64 = attached_size(store, &draft);
    let total = carried.saturating_add(bytes.len() as u64);
    if total > ATTACHMENT_BUDGET {
        return Err(format!(
            "that would make {} of attachments, and most servers refuse above {}. \
             Send a link instead, or split the message",
            crate::attach::human_size(total),
            crate::attach::human_size(ATTACHMENT_BUDGET),
        ));
    }

    let blob = store
        .blobs()
        .put(&store.connection(), bytes)
        .map_err(|e| format!("cannot store {name}: {e}"))?;
    draft.attachments.push(PendingAttachment {
        mime: media_type_of(&name).to_owned(),
        name,
        blob,
    });
    draft.updated = now;
    save(store, &draft)?;
    Ok(draft)
}

/// Read `path` and attach it.
pub fn attach_file(
    store: &SqliteStore,
    draft: DraftId,
    path: &std::path::Path,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    // Checked before reading, so attaching a 4 GB file is an error rather than 4 GB of memory.
    let size = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if size > ATTACHMENT_BUDGET {
        return Err(format!(
            "{} is {}, and most servers refuse above {}",
            path.display(),
            crate::attach::human_size(size),
            crate::attach::human_size(ATTACHMENT_BUDGET),
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
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
) -> Result<Draft, String> {
    let mut draft = store.draft(draft).map_err(|e| e.to_string())?;
    if index >= draft.attachments.len() {
        return Err(format!(
            "that draft has {} attachment(s); there is no number {index}",
            draft.attachments.len()
        ));
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
        .filter_map(|a| store.blobs().size(&store.connection(), a.blob).ok())
        .sum()
}

/// What a draft carries, as `(name, size)` ready to show.
///
/// The sizes come from the store rather than from whatever was just handed over, so a file that
/// failed to store cannot appear in the list as though it had.
pub fn attached_to(store: &SqliteStore, draft: &Draft) -> Vec<(String, String)> {
    draft
        .attachments
        .iter()
        .map(|a| {
            let size = store
                .blobs()
                .size(&store.connection(), a.blob)
                .map(crate::attach::human_size)
                .unwrap_or_else(|_| "missing".to_owned());
            (a.name.clone(), size)
        })
        .collect()
}

/// What a draft is carrying, as the CLI prints it.
pub fn attachments_of(store: &SqliteStore, draft: DraftId) -> Result<String, String> {
    let draft = store.draft(draft).map_err(|e| e.to_string())?;
    if draft.attachments.is_empty() {
        return Ok("nothing attached to that draft\n".to_owned());
    }
    let mut out = String::new();
    for (index, attachment) in draft.attachments.iter().enumerate() {
        let size = store
            .blobs()
            .size(&store.connection(), attachment.blob)
            .map(crate::attach::human_size)
            .unwrap_or_else(|_| "missing".to_owned());
        let _ = writeln!(
            out,
            "  {index}  {:>9}  {}  {}",
            size, attachment.mime, attachment.name
        );
    }
    Ok(out)
}

/// Send this draft from a different account.
///
/// Both columns move together. `identity` is a foreign key into the *new* account's identities,
/// so writing one without the other leaves a draft that `identity_of` cannot resolve and nothing
/// can send — and the failure would arrive at Send, long after the choice was made. The identity
/// is therefore looked up before anything is written, so an account that has none leaves the
/// draft exactly where it was.
pub fn move_draft_to(
    store: &SqliteStore,
    draft: DraftId,
    account: AccountId,
    now: DateTime<Utc>,
) -> Result<Draft, String> {
    let mut draft = store.draft(draft).map_err(|e| e.to_string())?;
    if draft.account == account {
        return Ok(draft);
    }
    let identity = identity_of(store, account, None)?;
    draft.account = account;
    draft.identity = identity.id;
    draft.updated = now;
    save(store, &draft)?;
    Ok(draft)
}

/// Start a new message, as the CLI reports it.
pub fn new_message(
    store: &SqliteStore,
    from: Option<&str>,
    to: &[Address],
    subject: &str,
    body: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let account = account_for(store, from)?;
    let draft = draft_new(store, account, to, subject, body, now)?;
    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    let _ = writeln!(
        out,
        "  subject {}",
        if draft.subject.is_empty() {
            "(none)"
        } else {
            &draft.subject
        }
    );
    let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    Ok(out)
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
    out.push_str("\r\n---------- Forwarded message ----------\r\n");
    let _ = write!(
        out,
        "From: {}\r\n",
        addresses(std::slice::from_ref(&original.from))
    );
    let _ = write!(
        out,
        "Date: {}\r\n",
        crate::view::stamp(original.date, zone, crate::view::Stamp::Quote)
    );
    let _ = write!(out, "Subject: {}\r\n", original.subject);
    // `To` and `Cc` as they were. Omitted when empty rather than written as a blank header,
    // which is what a header block copied from a message with no recipients would otherwise say.
    if !original.to.is_empty() {
        let _ = write!(out, "To: {}\r\n", addresses(&original.to));
    }
    if !original.cc.is_empty() {
        let _ = write!(out, "Cc: {}\r\n", addresses(&original.cc));
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

/// Forward a message, as the CLI reports it.
pub fn forward(
    store: &SqliteStore,
    message: MessageId,
    to: &[Address],
    body: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let draft = draft_forward(store, message, to, body, now)?;
    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    let _ = writeln!(out, "  subject {}", draft.subject);
    let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    Ok(out)
}

/// Write a draft back to the store.
///
/// Used by the composer on every save. `INSERT OR REPLACE` underneath, so this is also what an
/// autosave calls: the draft row is the document, and the widgets are only a view of it.
pub fn save(store: &SqliteStore, draft: &Draft) -> Result<(), String> {
    store
        .apply(
            draft.account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .map_err(|e| e.to_string())
}

/// Delete a draft.
///
/// There was no way to do this anywhere in the application. The composer's "Discard" only closed
/// the pane without saving — which for a draft that had never been saved is the same thing, and
/// for every other draft is not: a reply is persisted the moment it is created, and one opened
/// from the drafts list came off disk, so "Discard" left it sitting in Drafts for ever. Drafts
/// could be made and never unmade.
///
/// Returns the subject, so the caller can say what went.
pub fn discard(store: &SqliteStore, draft: DraftId) -> Result<String, String> {
    let draft = store.draft(draft).map_err(|e| e.to_string())?;
    // Mid-flight. Deleting the row would leave the outbox draining something that is no longer
    // there — and `Sending` in particular may already be on the wire, where nothing here can
    // recall it.
    if matches!(draft.state, SendState::Queued | SendState::Sending) {
        return Err(
            "that draft is queued for delivery; it cannot be discarded until the send settles"
                .to_owned(),
        );
    }
    store
        .apply(
            draft.account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftDelete(draft.id)],
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(draft.subject)
}

/// Start a reply to `message`, with `body` as its text.
pub fn reply(
    store: &SqliteStore,
    message: MessageId,
    scope: ReplyScope,
    body: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let draft = draft_reply(store, message, scope, body, now)?;

    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    if !draft.cc.is_empty() {
        let _ = writeln!(out, "  cc      {}", addresses(&draft.cc));
    }
    let _ = writeln!(out, "  subject {}", draft.subject);
    if draft.to.is_empty() && draft.cc.is_empty() {
        // `Draft::reply_to` drops your own address from the recipients, which is right — a reply
        // to something you sent has nobody left to go to. Saying "send it with: …" anyway meant
        // the next command failed with "cannot build a message with no recipients", and the CLI
        // has no way to add one, so the advice was not merely useless but unfollowable.
        let _ = writeln!(
            out,
            "\nnobody to send this to: the only address on the original was your own. \
             Open it in the composer to add a recipient."
        );
    } else {
        let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    }
    Ok(out)
}

/// Put the identity's signature beneath what was written.
///
/// `Identity.signature` has been a column since phase 1 and nothing has ever read it: no sending
/// path appended one, no composer showed one, and no command could set one. Every message this
/// client has sent went out unsigned.
///
/// Appended at compose time rather than at send, like the quoted material beside it, so the user
/// can see it, edit it, or take it off for one message. A signature the user cannot delete from
/// a particular reply is worse than none.
///
/// The delimiter is `"-- "` on its own line — two hyphens, a space, and nothing else. RFC 3676
/// §4.3 names that exact string, and every client that trims a signature when quoting looks for
/// it; `--` without the trailing space is a different line and gets quoted back at people for
/// the rest of the thread.
fn signed(body: &str, identity: &Identity) -> String {
    let Some(signature) = identity.signature.as_deref().map(str::trim_end) else {
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

/// Set or clear the signature on an account's default identity.
pub fn set_signature(
    store: &SqliteStore,
    account: AccountId,
    signature: Option<&str>,
) -> Result<String, String> {
    let identity = identity_of(store, account, None)?;
    // An empty string is not a signature: stored as NULL, so "has one" is a single question
    // rather than two that can disagree.
    let trimmed = signature.map(str::trim_end).filter(|s| !s.is_empty());
    store
        .connection()
        .execute(
            "UPDATE identities SET signature = ?2 WHERE id = ?1",
            rusqlite::params![identity.id.to_string(), trimmed],
        )
        .map_err(|e| e.to_string())?;
    Ok(match trimmed {
        Some(_) => format!("signature set for {}\n", identity.from.email),
        None => format!("signature cleared for {}\n", identity.from.email),
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
    let who = original
        .from
        .name
        .clone()
        .unwrap_or_else(|| original.from.email.clone());
    // In the sender's own zone. Quoted in UTC this said "at 01:02" for a message written at
    // 09:02, in a line the recipient reads and cannot correct.
    let when = crate::view::stamp(original.date, zone, crate::view::Stamp::Quote);
    let mut out = String::new();
    if !body.is_empty() {
        out.push_str(body.trim_end());
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    let _ = write!(out, "On {when}, {who} wrote:\r\n");
    // The text part, when there is one. A message whose body was never fetched quotes nothing
    // rather than quoting the word "None".
    if let Body::Present {
        text: Some(text), ..
    } = &original.body
    {
        for line in text.lines() {
            let _ = write!(out, "> {line}\r\n");
        }
    }
    out
}

/// Queue a draft for delivery.
///
/// Builds the bytes and the envelope together, freezes the bytes in the blob store, and puts a
/// submission in the outbox. Nothing here touches the network: the next `mailo sync` delivers
/// it, and until it does the message is safe across a restart.
pub fn send(store: &SqliteStore, draft: DraftId, now: DateTime<Utc>) -> Result<String, String> {
    let draft = store.draft(draft).map_err(|e| e.to_string())?;
    if let SendState::Sent { at, .. } = draft.state {
        return Err(format!("that draft was already sent at {at}"));
    }
    let identity = identity_of(store, draft.account, Some(draft.identity))?;
    let parent = draft.in_reply_to.and_then(|id| store.message(id).ok());

    // Attachment bytes come from the blob store, because `posting` is pure and takes them as
    // an argument rather than reading anything.
    let mut parts = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        let bytes = store
            .blobs()
            .get(&store.connection(), attachment.blob)
            .map_err(|e| format!("attachment {}: {e}", attachment.name))?;
        parts.push((attachment.blob, bytes));
    }

    let post = posting(&draft, &identity, parent.as_ref(), &parts).map_err(|e| e.to_string())?;
    let raw = store
        .blobs()
        .put(&store.connection(), &post.message)
        .map_err(|e| e.to_string())?;

    let queued = store
        .enqueue(
            draft.account,
            RemoteIntent::Send {
                draft: draft.id,
                raw,
                mail_from: post.mail_from.clone(),
                rcpt_to: post.rcpt_to.clone(),
            },
            // No undo. Unsending is not something the outbox can offer once the bytes are on
            // the wire, and a patch that pretended otherwise would revert something real.
            &Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            },
            now,
        )
        .map_err(|e| e.to_string())?;
    if queued.is_none() {
        return Err("the submission could not be queued".to_owned());
    }
    store
        .set_send_state(draft.id, &SendState::Queued, now)
        .map_err(|e| e.to_string())?;

    let mut out = format!("queued {} for delivery\n", draft.id);
    let _ = writeln!(out, "  from    {}", post.mail_from);
    let _ = writeln!(out, "  to      {}", post.rcpt_to.join(", "));
    let _ = writeln!(out, "  subject {}", draft.subject);
    let _ = writeln!(out, "\ndeliver it with: mailo sync");
    Ok(out)
}

/// Every draft, and where it got to.
pub fn drafts(store: &SqliteStore) -> Result<String, String> {
    let accounts: Vec<AccountId> = {
        let db = store.connection();
        let mut stmt = db
            .prepare("SELECT id FROM accounts ORDER BY created_at")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(id) = row.map_err(|e| e.to_string())?.parse() {
                out.push(AccountId::from_uuid(id));
            }
        }
        out
    };

    let mut out = String::new();
    for account in accounts {
        for draft in store.drafts(account).map_err(|e| e.to_string())? {
            let _ = writeln!(
                out,
                "{}  {:<9}  {}",
                draft.id,
                state_word(&draft.state),
                if draft.subject.is_empty() {
                    "(no subject)"
                } else {
                    &draft.subject
                }
            );
        }
    }
    if out.is_empty() {
        out.push_str("no drafts.\n");
    }
    Ok(out)
}

fn state_word(state: &SendState) -> &'static str {
    match state {
        SendState::Editing => "editing",
        SendState::Queued => "queued",
        SendState::Sending => "sending",
        SendState::Failed { .. } => "failed",
        SendState::Sent { .. } => "sent",
    }
}

fn addresses(list: &[Address]) -> String {
    list.iter()
        .map(|a| match &a.name {
            Some(name) => format!("{name} <{}>", a.email),
            None => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
