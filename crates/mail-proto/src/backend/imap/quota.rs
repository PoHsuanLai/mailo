//! An IMAP account's storage, from its server's `QUOTA` extension (RFC 9208).
//!
//! Not a [`ProtoOp`](mail_domain::ProtoOp), for the reason a search is not one: it changes
//! nothing, is never queued, and its answer is no [`ProtoOutcome`](crate::ProtoOutcome). It is a
//! machine of its own over the backend's session factory, so it signs in as every walk does.
//!
//! The walk is `CAPABILITY` (the list after sign-in is the one that counts), a guard that stops
//! it unless the server offers `QUOTA`, and `GETQUOTAROOT INBOX`: every root the inbox counts
//! against, with each root's usage. A server without the extension is no failure: it has no
//! limit to say, and the walk ends with `None` without sending the command.

use super::{Authenticate, ImapBackend};
use crate::imap::{ImapCommand, ImapSession, ImapTranscript, has_capability};
use crate::machine::{IoReady, Machine, Progress, ProtoError};
use crate::quota::{Storage, tightest};
use imap_proto::{QuotaResourceName, Response};

/// The extension the walk needs.
const QUOTA: &str = "QUOTA";

/// The reading of an account's storage, as a [`Machine`] the runtime drives over one connection.
pub struct Quota<'a> {
    backend: &'a mut ImapBackend,
    session: Option<ImapSession>,
}

// By hand: the backend holds a session factory, which has no Debug and holds a credential.
impl std::fmt::Debug for Quota<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Quota").finish_non_exhaustive()
    }
}

impl ImapBackend {
    /// A walk that reads how much of the account's storage is used: `None` when the server
    /// offers no quota, or counts none against the inbox.
    pub fn quota(&mut self) -> Quota<'_> {
        Quota {
            backend: self,
            session: None,
        }
    }
}

/// The tightest storage limit among the `QUOTA` responses in `transcript`.
///
/// `STORAGE` counts units of 1024 octets (RFC 9208 §5.1). Read from the parsed response, not its
/// text, as every answer that decides something is here.
fn storage(transcript: &ImapTranscript) -> Option<Storage> {
    tightest(transcript.untagged.iter().flat_map(|u| {
        match imap_proto::parser::parse_response(&u.raw) {
            Ok((_, Response::Quota(quota))) => quota
                .resources
                .into_iter()
                .filter(|r| r.name == QuotaResourceName::Storage)
                .map(|r| Storage {
                    used: r.usage.saturating_mul(1024),
                    limit: r.limit.saturating_mul(1024),
                })
                .collect(),
            _ => Vec::new(),
        }
    }))
}

impl Machine for Quota<'_> {
    type Out = Option<Storage>;

    fn start(&mut self) -> Progress<Option<Storage>> {
        let commands = vec![
            ImapCommand::Capability,
            ImapCommand::RequireCapability(vec![QUOTA.to_owned()]),
            ImapCommand::GetQuotaRoot {
                mailbox: "INBOX".to_owned(),
            },
        ];
        let mut session = match (self.backend.build)(Authenticate::First, commands) {
            Ok(session) => session,
            Err(e) => return Progress::Failed(e),
        };
        let started = session.start();
        self.session = Some(session);
        match started {
            Progress::Need(needs) => Progress::Need(needs),
            Progress::Done(_) => Progress::Failed(ProtoError::Malformed(
                "session finished before doing anything".to_owned(),
            )),
            Progress::Failed(e) => Progress::Failed(e),
        }
    }

    fn feed(&mut self, ready: IoReady) -> Progress<Option<Storage>> {
        let Some(session) = self.session.as_mut() else {
            return Progress::Failed(ProtoError::Malformed(
                "bytes arrived with no quota asked".to_owned(),
            ));
        };
        match session.feed(ready) {
            Progress::Need(needs) => Progress::Need(needs),
            // The guard stopped the walk: no quota to read, which is an answer. Only the
            // guard's refusal names the extension; one at sign-in (`LOGINDISABLED`, a credential
            // of the wrong kind) is a failure still.
            Progress::Failed(ProtoError::Unsupported(why))
                if why.contains(QUOTA) && !has_capability(session.capabilities(), QUOTA) =>
            {
                Progress::Done(None)
            }
            Progress::Failed(e) => Progress::Failed(e),
            Progress::Done(transcript) => Progress::Done(storage(&transcript)),
        }
    }
}
