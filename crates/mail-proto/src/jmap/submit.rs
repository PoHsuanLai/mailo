//! Sending (RFC 8621 §6 and §7): an identity to send as, and a submission of uploaded bytes.
//!
//! The message goes up as the exact bytes frozen when the user pressed send, is filed in Drafts
//! by `Email/import`, and is submitted by `EmailSubmission/set` in the same request, naming the
//! imported email by its creation id. The envelope is given explicitly — `mailFrom` and every
//! `rcptTo` — because the bytes deliberately carry no `Bcc:` (FINDINGS F37), and a server that
//! derived recipients from the headers would drop every blind copy. On success the server moves
//! the email from Drafts to Sent and clears `$draft` itself (`onSuccessUpdateEmail`), so the
//! sent copy is filed where every other client looks for it.

use super::field::{malformed, opt_string, string};
use super::{Call, MethodError, Responses, SetResult};
use crate::ProtoError;
use serde_json::{Map, Value, json};

/// An address this account may send as (RFC 8621 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

impl Identity {
    /// Parse the `list` of an `Identity/get` answer.
    pub fn parse_list(args: &Value) -> Result<Vec<Identity>, ProtoError> {
        args.get("list")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("Identity/get has no list"))?
            .iter()
            .map(|i| {
                Ok(Identity {
                    id: string(i, "id")?.to_owned(),
                    email: string(i, "email")?.to_owned(),
                    name: opt_string(i, "name")?
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned),
                })
            })
            .collect()
    }
}

/// The identity to submit `mail_from` as: the one with that address, or else one that covers
/// its whole domain (`*@example.com`, RFC 8621 §6).
///
/// `None` when neither exists. The server would refuse such a submission (`forbiddenFrom`), and
/// sending as some other identity instead would put a From the user did not choose on the mail.
pub fn choose_identity<'a>(identities: &'a [Identity], mail_from: &str) -> Option<&'a Identity> {
    let exact = identities
        .iter()
        .find(|i| i.email.eq_ignore_ascii_case(mail_from));
    exact.or_else(|| {
        let (_, domain) = mail_from.rsplit_once('@')?;
        identities.iter().find(|i| {
            i.email
                .strip_prefix("*@")
                .is_some_and(|d| d.eq_ignore_ascii_case(domain))
        })
    })
}

/// Where the imported copy goes, and where it moves on success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    pub drafts: Option<String>,
    pub sent: Option<String>,
}

/// The two calls that send `blob`: import it, then submit it. Ids `import` and `send`.
///
/// Refused before anything is sent when the server lists neither a Drafts nor a Sent mailbox,
/// since an imported email must be filed somewhere.
pub fn submission(
    account: &str,
    blob: &str,
    identity: &Identity,
    mail_from: &str,
    rcpt_to: &[String],
    filed: &Filed,
) -> Result<Vec<Call>, ProtoError> {
    let into = filed
        .drafts
        .as_deref()
        .or(filed.sent.as_deref())
        .ok_or_else(|| {
            ProtoError::Unsupported(
                "sending over JMAP: the server lists no Drafts or Sent mailbox to file the \
                 message in"
                    .to_owned(),
            )
        })?;
    let mut mailbox_ids = Map::new();
    mailbox_ids.insert(into.to_owned(), json!(true));
    let import = Call {
        name: "Email/import",
        args: json!({
            "accountId": account,
            "emails": {
                "draft": {
                    "blobId": blob,
                    "mailboxIds": mailbox_ids,
                    "keywords": { "$draft": true, "$seen": true },
                }
            },
        }),
        id: "import".to_owned(),
    };
    let mut on_success = Map::new();
    on_success.insert("keywords/$draft".to_owned(), Value::Null);
    if let (Some(drafts), Some(sent)) = (&filed.drafts, &filed.sent) {
        on_success.insert(format!("mailboxIds/{drafts}"), Value::Null);
        on_success.insert(format!("mailboxIds/{sent}"), json!(true));
    }
    let mut update = Map::new();
    update.insert("#send".to_owned(), Value::Object(on_success));
    let send = Call {
        name: "EmailSubmission/set",
        args: json!({
            "accountId": account,
            "create": {
                "send": {
                    "identityId": identity.id,
                    "emailId": "#draft",
                    "envelope": {
                        "mailFrom": { "email": mail_from, "parameters": null },
                        "rcptTo": rcpt_to
                            .iter()
                            .map(|r| json!({ "email": r, "parameters": null }))
                            .collect::<Vec<_>>(),
                    },
                }
            },
            "onSuccessUpdateEmail": update,
        }),
        id: "send".to_owned(),
    };
    Ok(vec![import, send])
}

/// What a submission produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// The email the server now holds, filed in Sent.
    pub email_id: String,
    pub submission_id: String,
}

/// Read the answers to [`submission`]'s calls.
///
/// The import is read first: a refused import means nothing was submitted, and its reason
/// (`overQuota`, `tooLarge`, a blob that is not a message) is the one to report.
pub fn submitted(responses: &Responses) -> Result<Sent, ProtoError> {
    let imported = SetResult::parse(responses.answer("import", "Email/import")?)?;
    if let Some(e) = imported.first_refusal("importing the message") {
        return Err(e);
    }
    let email_id = created_id(&imported, "draft")?;
    let sent = SetResult::parse(responses.answer("send", "EmailSubmission/set")?)?;
    if let Some(e) = sent.first_refusal("submitting the message") {
        return Err(e);
    }
    let submission_id = created_id(&sent, "send")?;
    Ok(Sent {
        email_id,
        submission_id,
    })
}

fn created_id(result: &SetResult, creation: &str) -> Result<String, ProtoError> {
    result
        .created
        .iter()
        .find(|(k, _)| k == creation)
        .and_then(|(_, v)| v.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .ok_or_else(|| {
            ProtoError::from(MethodError::Missing(format!(
                "{creation} was neither created nor refused"
            )))
        })
}
