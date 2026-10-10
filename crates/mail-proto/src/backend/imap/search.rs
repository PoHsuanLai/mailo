//! A server search over IMAP: every mailbox of an [`ImapPlan`] searched on one connection.
//!
//! Not a [`ProtoOp`](mail_domain::ProtoOp): a search changes nothing, is never queued, and its
//! answer is UIDs, which no [`ProtoOutcome`](crate::ProtoOutcome) carries. It is a machine of its
//! own over the backend's session factory, so it signs in exactly as every other walk does.
//!
//! The walk is `CAPABILITY` (whether the server has `ESEARCH` and non-synchronizing literals is
//! decided from what it says after sign-in), then `EXAMINE` and `UID SEARCH` per mailbox:
//! read-only, so searching never sets `\Recent` or expunges anything.

use super::{Authenticate, ImapBackend, mailbox_state};
use crate::imap::{Access, ImapCommand, ImapSession, ImapTranscript, Untagged};
use crate::machine::{IoReady, Machine, Progress, ProtoError};
use crate::search::imap::{ImapPlan, hits};

/// What one mailbox's search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The mailbox, as the server names it.
    pub mailbox: String,
    /// The `UIDVALIDITY` its `EXAMINE` reported: what the UIDs mean.
    pub uidvalidity: u32,
    /// Every matching UID, ascending.
    pub uids: Vec<u32>,
    /// How many match, by the server's count where it gave one.
    pub count: u64,
}

/// The search of an [`ImapPlan`], as a [`Machine`] the runtime drives over one connection.
pub struct Searching<'a> {
    backend: &'a mut ImapBackend,
    plan: ImapPlan,
    session: Option<ImapSession>,
}

// By hand: the backend holds a session factory, which has no Debug and holds a credential.
impl std::fmt::Debug for Searching<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Searching")
            .field("plan", &self.plan)
            .finish_non_exhaustive()
    }
}

impl ImapBackend {
    /// A walk that searches every mailbox of `plan` and returns what each found.
    pub fn searching(&mut self, plan: ImapPlan) -> Searching<'_> {
        Searching {
            backend: self,
            plan,
            session: None,
        }
    }
}

impl Searching<'_> {
    /// Where each mailbox's `EXAMINE` and `UID SEARCH` land in the walk, after the capability.
    fn positions(&self) -> impl Iterator<Item = (usize, &String)> {
        self.plan
            .mailboxes
            .iter()
            .enumerate()
            .map(|(n, mailbox)| (1 + 2 * n, mailbox))
    }

    fn found(&self, transcript: &ImapTranscript) -> Result<Vec<Found>, ProtoError> {
        // The factory put its sign-in first: the walk's own commands start after it.
        let offset = transcript
            .completed
            .iter()
            .map(|c| c.index)
            .max()
            .map_or(0, |last| last + 1 - (1 + 2 * self.plan.mailboxes.len()));
        self.positions()
            .map(|(at, mailbox)| {
                let during = |index: usize| -> Vec<Untagged> {
                    transcript
                        .untagged
                        .iter()
                        .filter(|u| u.during == index)
                        .cloned()
                        .collect()
                };
                let (uidvalidity, _, _) = mailbox_state(&during(offset + at));
                let hits = hits(&transcript.untagged, offset + at + 1)?;
                Ok(Found {
                    mailbox: mailbox.clone(),
                    uidvalidity,
                    uids: hits.uids,
                    count: hits.count,
                })
            })
            .collect()
    }
}

impl Machine for Searching<'_> {
    type Out = Vec<Found>;

    fn start(&mut self) -> Progress<Vec<Found>> {
        if self.plan.mailboxes.is_empty() {
            return Progress::Done(Vec::new());
        }
        let mut commands = vec![ImapCommand::Capability];
        for mailbox in &self.plan.mailboxes {
            commands.push(ImapCommand::Select {
                mailbox: mailbox.clone(),
                access: Access::ReadOnly,
                qresync: None,
            });
            commands.push(ImapCommand::Search {
                keys: self.plan.keys.clone(),
            });
        }
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

    fn feed(&mut self, ready: IoReady) -> Progress<Vec<Found>> {
        let Some(session) = self.session.as_mut() else {
            return Progress::Failed(ProtoError::Malformed(
                "bytes arrived with no search in flight".to_owned(),
            ));
        };
        match session.feed(ready) {
            Progress::Need(needs) => Progress::Need(needs),
            Progress::Failed(e) => Progress::Failed(e),
            Progress::Done(transcript) => match self.found(&transcript) {
                Ok(found) => Progress::Done(found),
                Err(e) => Progress::Failed(e),
            },
        }
    }
}
