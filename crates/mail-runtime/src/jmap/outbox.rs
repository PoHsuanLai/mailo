//! Draining the outbox to a JMAP server: the same operations, in the same order, with the same
//! settle and undo rules as [`crate::AccountEngine::drain_outbox`].
//!
//! Each operation is one request. Flags, filing, labels and keywords are `Email/set` patches;
//! an upload is `Email/import`; a send is an import and an `EmailSubmission/set` together;
//! folder work is `Mailbox/set`. Destroying an email is done for one reason only — the user
//! emptied the Trash — and is checked against the server's own view before it is asked for.

use super::{Client, JmapEngine, email_ids};
use crate::{RuntimeError, SyncReport};
use chrono::{DateTime, Utc};
use mail_domain::{
    BlobId, MailboxRef, MailboxRole, ProtoOp, RemoteRef, Retry, Retryable, SendState, SystemFlag,
};
use mail_proto::jmap::{
    self, CORE, EmailSummary, Filed, Identity, Ids, MAIL, SUBMISSION, SetResult, Submission,
};
use mail_proto::{ProtoError, ProtoOutcome, Refusal};
use mail_store::{Dispatch, Settle, Store};
use serde_json::{Map, Value};

impl JmapEngine {
    /// Drain the outbox into `report`, stopping at the first failure so nothing overtakes it.
    pub(super) async fn drain_into(
        &mut self,
        now: DateTime<Utc>,
        report: &mut SyncReport,
    ) -> Result<(), RuntimeError> {
        for entry in self.store.outbox_due(self.account, now)? {
            let id = entry.id;
            // Addressed as it is reached, as `AccountEngine::drain_outbox` does. A JMAP id does
            // not change when an email moves, so this finds the same one; it matters only for a
            // message this client no longer holds, or holds at no address.
            let op = match self.store.outbox_dispatch(id)? {
                Dispatch::Send(op) => op,
                Dispatch::Wait => continue,
                Dispatch::Moot => {
                    self.store.outbox_settle(id, Settle::Ok, now)?;
                    continue;
                }
                // Not reached: only an IMAP move leaves a message with no address. Refused all
                // the same, as `AccountEngine::drain_outbox` refuses it, rather than kept for good.
                Dispatch::Lost(reason) => {
                    crate::engine::given_up(&*self.store, id, reason, now, report)?;
                    continue;
                }
            };
            let entry = mail_store::OutboxEntry { op, ..entry };
            let draft = match &entry.op {
                ProtoOp::Submit { draft, .. } => Some(*draft),
                _ => None,
            };
            let upload = match &entry.op {
                ProtoOp::Append {
                    mailbox,
                    flags,
                    raw,
                    ..
                } => Some((mailbox.clone(), flags.clone(), *raw)),
                _ => None,
            };
            if let Some(draft) = draft {
                self.mark_draft(draft, SendState::Sending, now);
            }
            match self.run(entry.op, draft.map(|_| now)).await {
                Ok(outcome) => {
                    self.store.outbox_settle(id, Settle::Ok, now)?;
                    if let (
                        Some(upload),
                        ProtoOutcome::Appended {
                            remote: Some(remote),
                        },
                    ) = (upload, &outcome)
                    {
                        report.appended += 1;
                        if let Err(e) = self.keep_appended(upload, remote.clone(), now) {
                            report.needs_attention.push(format!(
                                "uploaded, but not kept here until the next sync: {e}"
                            ));
                        }
                    }
                    if let Some(draft) = draft {
                        self.mark_draft(
                            draft,
                            SendState::Sent {
                                at: now,
                                message: None,
                            },
                            now,
                        );
                        report.submitted += 1;
                    }
                    report.outbox_settled += 1;
                }
                Err(e) => {
                    self.forget_if_stale(&e);
                    let retry = e.retry();
                    if matches!(retry, Retry::NeedsReauth | Retry::Fatal(_)) {
                        report.needs_attention.push(e.to_string());
                    }
                    report.saw(&retry);
                    if let Some(draft) = draft {
                        self.mark_draft(
                            draft,
                            SendState::Failed {
                                reason: e.to_string(),
                                retry: retry.clone(),
                            },
                            now,
                        );
                    }
                    self.store.outbox_settle(
                        id,
                        Settle::Failed {
                            reason: e.to_string(),
                            retry,
                        },
                        now,
                    )?;
                    break;
                }
            }
        }
        report.still_queued = self
            .store
            .outbox_due(
                self.account,
                now + chrono::TimeDelta::try_days(365).unwrap_or_default(),
            )
            .map(|due| due.len())
            .unwrap_or(0);
        Ok(())
    }

    fn mark_draft(&self, draft: mail_domain::DraftId, state: SendState, now: DateTime<Utc>) {
        // As in `AccountEngine::mark_draft`: a draft deleted while queued is nobody's concern.
        let _ = self.store.set_send_state(draft, &state, now);
    }

    /// Keep a message this client just uploaded, at the id the server gave it.
    fn keep_appended(
        &self,
        (mailbox, flags, raw): (MailboxRef, Vec<SystemFlag>, BlobId),
        remote: RemoteRef,
        now: DateTime<Utc>,
    ) -> Result<(), RuntimeError> {
        let bytes = self.store.blobs().get(&self.store.connection(), raw)?;
        let role = self
            .mailboxes
            .as_ref()
            .map_or(MailboxRole::Archive, |m| m.roles().filed_as(&mailbox.path));
        crate::assemble::appended(
            &self.store,
            self.account,
            crate::assemble::Destination {
                mailbox: self.mailbox(),
                role,
            },
            remote,
            bytes,
            &flags,
            now,
        )?;
        Ok(())
    }

    /// Do one operation. `leaving` is when a submission leaves, stamped into its `Date`.
    async fn run(
        &mut self,
        op: ProtoOp,
        leaving: Option<DateTime<Utc>>,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let client = self.client().await?.clone();
        let mailboxes = match &self.mailboxes {
            Some(m) => m.clone(),
            None => self.refresh_mailboxes(Utc::now()).await?,
        };
        let account = client.session.account.clone();
        let patch_each = |remotes: &[RemoteRef], patch: Map<String, Value>| {
            email_ids(remotes)
                .into_iter()
                .map(|id| (id, patch.clone()))
                .collect::<Vec<_>>()
        };
        match op {
            ProtoOp::SetFlags {
                remotes,
                read,
                star,
            } => {
                let update = patch_each(&remotes, jmap::flags_patch(read, star));
                self.set(&client, update, Vec::new()).await
            }
            ProtoOp::SetMailbox { remotes, role } => {
                let update = patch_each(&remotes, jmap::filing_patch(role, &mailboxes)?);
                self.set(&client, update, Vec::new()).await
            }
            ProtoOp::SetLabels {
                remotes,
                add,
                remove,
            } => {
                // A label made here a moment ago is a mailbox the queue created just before;
                // the list this engine holds predates it, so look again before refusing.
                let mailboxes = if add.iter().all(|n| mailboxes.id_for_path(n).is_some()) {
                    mailboxes
                } else {
                    self.mailboxes = None;
                    self.refresh_mailboxes(Utc::now()).await?
                };
                let update = patch_each(&remotes, jmap::labels_patch(&add, &remove, &mailboxes)?);
                self.set(&client, update, Vec::new()).await
            }
            ProtoOp::AddKeyword { remotes, keyword } => {
                let update = patch_each(&remotes, jmap::keyword_patch(keyword));
                self.set(&client, update, Vec::new()).await
            }
            ProtoOp::Expunge { remotes } => {
                let doomed = self.in_trash_only(&client, &mailboxes, &remotes).await?;
                self.set(&client, Vec::new(), doomed).await
            }
            ProtoOp::Append {
                mailbox,
                flags,
                date,
                raw,
            } => {
                let target = match mailboxes.id_for_path(&mailbox.path) {
                    Some(id) => id.to_owned(),
                    None => {
                        return Err(permanent(format!(
                            "the server has no mailbox called {:?}",
                            mailbox.path
                        )));
                    }
                };
                let bytes = self.store.blobs().get(&self.store.connection(), raw)?;
                let blob = client.upload(bytes).await?;
                let keywords: Vec<&str> = flags.iter().map(keyword_of).collect();
                let call = jmap::email_import(&account, &blob, &target, &keywords, date, "i");
                let responses = client.call(&[CORE, MAIL], &[call]).await?;
                let result = SetResult::parse(responses.answer("i", "Email/import")?)?;
                if let Some(e) = result.first_refusal("uploading the message") {
                    return Err(e.into());
                }
                let remote = result
                    .created
                    .iter()
                    .find_map(|(_, v)| v.get("id").and_then(Value::as_str))
                    .map(|id| RemoteRef::Jmap {
                        email_id: id.to_owned(),
                    });
                Ok(ProtoOutcome::Appended { remote })
            }
            ProtoOp::Submit {
                raw,
                mail_from,
                rcpt_to,
                ..
            } => {
                self.submit(&client, &mailboxes, raw, &mail_from, &rcpt_to, leaving)
                    .await
            }
            ProtoOp::Folder(work) => {
                let args = jmap::folder_work(&work, &mailboxes)?;
                let call = jmap::mailbox_set(&account, args, "f");
                let responses = client.call(&[CORE, MAIL], &[call]).await?;
                let result = SetResult::parse(responses.answer("f", "Mailbox/set")?)?;
                if let Some(e) = result.first_refusal("changing the folder") {
                    return Err(e.into());
                }
                // The list changed; the next use lists it again.
                self.mailboxes = None;
                Ok(ProtoOutcome::Applied)
            }
            other => Err(RuntimeError::UnsupportedIo(format!(
                "a JMAP account does not queue {other:?}"
            ))),
        }
    }

    /// One `Email/set`, succeeding when every email was changed or is already gone.
    async fn set(
        &self,
        client: &Client,
        update: Vec<(String, Map<String, Value>)>,
        destroy: Vec<String>,
    ) -> Result<ProtoOutcome, RuntimeError> {
        if update.is_empty() && destroy.is_empty() {
            return Ok(ProtoOutcome::Applied);
        }
        let account = client.session.account.as_str();
        let call = jmap::email_set(account, update, destroy, "s");
        let responses = client.call(&[CORE, MAIL], &[call]).await?;
        let result = SetResult::parse(responses.answer("s", "Email/set")?)?;
        match result.first_refusal("changing the message") {
            Some(e) => Err(e.into()),
            None => Ok(ProtoOutcome::Applied),
        }
    }

    /// Which of `remotes` the server holds in the Trash and nowhere else.
    ///
    /// The only emails this client will destroy. An email that is also in the inbox or under a
    /// label is not the user's rubbish, whatever this client last believed.
    async fn in_trash_only(
        &self,
        client: &Client,
        mailboxes: &jmap::Mailboxes,
        remotes: &[RemoteRef],
    ) -> Result<Vec<String>, RuntimeError> {
        let Some(trash) = mailboxes.id_for_role(MailboxRole::Trash) else {
            return Ok(Vec::new());
        };
        let ids = email_ids(remotes);
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let account = client.session.account.as_str();
        let call = jmap::email_get(account, Ids::Listed(ids), &["id", "mailboxIds"], "t");
        let responses = client.call(&[CORE, MAIL], &[call]).await?;
        Ok(
            EmailSummary::parse_list(responses.answer("t", "Email/get")?)?
                .into_iter()
                .filter(|e| e.mailbox_ids.iter().all(|m| m == trash) && !e.mailbox_ids.is_empty())
                .map(|e| e.id)
                .collect(),
        )
    }

    /// Send one frozen message: upload it, import it into Drafts, submit it.
    async fn submit(
        &mut self,
        client: &Client,
        mailboxes: &jmap::Mailboxes,
        raw: BlobId,
        mail_from: &str,
        rcpt_to: &[String],
        leaving: Option<DateTime<Utc>>,
    ) -> Result<ProtoOutcome, RuntimeError> {
        if client.session.submission == Submission::Absent {
            return Err(ProtoError::Unsupported(
                "this JMAP account may not send (no urn:ietf:params:jmap:submission)".to_owned(),
            )
            .into());
        }
        let identities = self.identities(client).await?;
        let identity = jmap::choose_identity(&identities, mail_from).ok_or_else(|| {
            permanent(format!(
                "the server lists no identity for {mail_from}, so it would refuse to send as it"
            ))
        })?;
        let frozen = self.store.blobs().get(&self.store.connection(), raw)?;
        let message = match leaving {
            Some(at) => mail_mime::restamp(&frozen, at),
            None => frozen,
        };
        let blob = client.upload(message).await?;
        let filed = Filed {
            drafts: mailboxes
                .id_for_role(MailboxRole::Drafts)
                .map(str::to_owned),
            sent: mailboxes.id_for_role(MailboxRole::Sent).map(str::to_owned),
        };
        let calls = jmap::submission(
            &client.session.account,
            &blob,
            identity,
            mail_from,
            rcpt_to,
            &filed,
        )?;
        let responses = client.call(&[CORE, MAIL, SUBMISSION], &calls).await?;
        let sent = jmap::submitted(&responses)?;
        Ok(ProtoOutcome::Submitted {
            remote: Some(RemoteRef::Jmap {
                email_id: sent.email_id,
            }),
        })
    }

    /// The identities this account may send as, asked for once per engine.
    async fn identities(&mut self, client: &Client) -> Result<Vec<Identity>, RuntimeError> {
        if let Some(known) = &self.identities {
            return Ok(known.clone());
        }
        let call = jmap::identity_get(&client.session.account, "i");
        let responses = client.call(&[CORE, SUBMISSION], &[call]).await?;
        let identities = Identity::parse_list(responses.answer("i", "Identity/get")?)?;
        self.identities = Some(identities.clone());
        Ok(identities)
    }
}

fn permanent(text: String) -> RuntimeError {
    RuntimeError::Proto(ProtoError::Refused {
        kind: Refusal::Permanent,
        text,
    })
}

/// The JMAP keyword for an IMAP system flag (RFC 8621 §4.1.1).
fn keyword_of(flag: &SystemFlag) -> &'static str {
    match flag {
        SystemFlag::Seen => "$seen",
        SystemFlag::Answered => "$answered",
        SystemFlag::Flagged => "$flagged",
        SystemFlag::Draft => "$draft",
    }
}
