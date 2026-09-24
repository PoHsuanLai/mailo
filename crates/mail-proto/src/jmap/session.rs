//! The session resource (RFC 8620 §2): where everything else is, and what the server allows.

use super::field::{malformed, object, opt_string, string, unsigned};
use super::{MAIL, SUBMISSION};
use crate::ProtoError;
use serde_json::Value;

/// What a JMAP session resource said, reduced to what this client uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Where every method call is posted.
    pub api_url: String,
    /// A URI template (RFC 6570 level 1) with `accountId`, `blobId`, `type` and `name`.
    pub download_url: String,
    /// A URI template with `accountId`.
    pub upload_url: String,
    /// A URI template with `types`, `closeafter` and `ping`; `None` when the server offers no
    /// push, and then the account is polled.
    pub event_source_url: Option<String>,
    /// The account holding the user's mail: the primary account for the mail capability.
    pub account: String,
    /// Whether that account may send (`urn:ietf:params:jmap:submission`).
    pub submission: Submission,
    pub limits: Limits,
    /// The name the server knows the user by.
    pub username: String,
    /// Changes when anything above does; a response carrying a different one means this
    /// session should be fetched again.
    pub state: String,
}

/// Whether the mail account can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    Offered,
    Absent,
}

/// The core capability's limits this client has to stay under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most ids one `/get` may name. RFC 8620 requires at least 500 to be *recommended*; we
    /// take what the server says.
    pub max_objects_in_get: usize,
    /// Most method calls in one request.
    pub max_calls_in_request: usize,
    /// Largest blob one upload may carry, in octets.
    pub max_size_upload: u64,
}

impl Session {
    /// Parse a session resource.
    ///
    /// Refused when the server lists no account for mail: a JMAP server can hold contacts or
    /// calendars alone, and treating one of those as a mail account would sync nothing, quietly.
    pub fn parse(bytes: &[u8]) -> Result<Session, ProtoError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| malformed(format!("the session is not JSON: {e}")))?;
        object(&value, "the session")?;
        let account = value
            .get("primaryAccounts")
            .and_then(|p| p.get(MAIL))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProtoError::Unsupported(
                    "JMAP mail: the session names no account for mail (no primary account for \
                     urn:ietf:params:jmap:mail)"
                        .to_owned(),
                )
            })?
            .to_owned();
        let account_caps = value
            .get("accounts")
            .and_then(|a| a.get(&account))
            .and_then(|a| a.get("accountCapabilities"));
        let submission = match account_caps.and_then(|c| c.get(SUBMISSION)) {
            Some(_) => Submission::Offered,
            None => Submission::Absent,
        };
        let core = value
            .get("capabilities")
            .and_then(|c| c.get(super::CORE))
            .ok_or_else(|| malformed("the session lists no core capability"))?;
        let limits = Limits {
            max_objects_in_get: clamp(unsigned(core, "maxObjectsInGet", 256)?),
            max_calls_in_request: clamp(unsigned(core, "maxCallsInRequest", 16)?),
            max_size_upload: unsigned(core, "maxSizeUpload", 0)?,
        };
        Ok(Session {
            api_url: string(&value, "apiUrl")?.to_owned(),
            download_url: string(&value, "downloadUrl")?.to_owned(),
            upload_url: string(&value, "uploadUrl")?.to_owned(),
            event_source_url: opt_string(&value, "eventSourceUrl")?
                .filter(|u| !u.is_empty())
                .map(str::to_owned),
            account,
            submission,
            limits,
            username: opt_string(&value, "username")?.unwrap_or("").to_owned(),
            state: opt_string(&value, "state")?.unwrap_or("").to_owned(),
        })
    }
}

/// A limit as a count we can use: at least one, and no more than this client would ever ask.
///
/// A server that says zero would otherwise make every batch empty and every loop endless.
fn clamp(n: u64) -> usize {
    n.clamp(1, 4096) as usize
}
