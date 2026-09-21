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
    draft.text = quoted(body, &original, zone);
    save(store, &draft)?;
    Ok(draft)
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
