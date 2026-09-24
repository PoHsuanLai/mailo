//! Bringing the store up to date with a JMAP account.
//!
//! A pass asks `Email/changes` what moved since the stored state, and applies it: new emails are
//! fetched, changed ones refiled and reflagged, destroyed ones dropped. It then asks how many
//! emails the server holds, and when that is not the number held here — a first sync, a
//! backfill still under way, a state too old to report changes from — it lists them all and
//! takes the difference, which is the one answer that is always right. Headers come first, the
//! bodies behind them, as on every protocol.

use super::{Client, JmapEngine, email_ids};
use crate::assemble::{Arrival, Destination, absorb_into};
use crate::{RuntimeError, SyncReport};
use chrono::{DateTime, Utc};
use mail_domain::{Ingest, MailboxRole, ReadState, RemoteRef, Star, SyncCursor, UidValidity};
use mail_proto::ProtoError;
use mail_proto::jmap::{
    self, CORE, Changes, EmailSummary, Filing, Ids, MAIL, Mailboxes, MethodError, More, QueryPage,
};
use mail_store::Store;
use std::collections::HashSet;

/// Most changes asked for in one call. The server may cap it lower.
const CHANGES_PER_CALL: u64 = 256;
/// Most rounds of `Email/changes` in one pass. The state reached is kept, so a pass that stops
/// here resumes from where it got to.
const CHANGE_ROUNDS: usize = 64;
/// Ids asked for per page of a full listing. The server may return fewer.
const PAGE: u64 = 1024;

/// What the server said about emails already held.
#[derive(Debug, Default)]
struct Truth {
    flags: Vec<(RemoteRef, ReadState, Star)>,
    labels: Vec<(RemoteRef, Vec<String>)>,
    filed: Vec<(RemoteRef, MailboxRole)>,
    gone: Vec<RemoteRef>,
}

impl Truth {
    /// Record one email as the server described it, returning whether it is one to hold.
    fn saw(&mut self, email: &EmailSummary, mailboxes: &Mailboxes) -> Option<MailboxRole> {
        let remote = RemoteRef::Jmap {
            email_id: email.id.clone(),
        };
        match jmap::filing(&email.mailbox_ids, mailboxes) {
            // Only in Drafts or Junk now: no longer followed, so gone from here.
            Filing::Unfollowed => {
                self.gone.push(remote);
                None
            }
            Filing::Filed { role, labels } => {
                self.flags
                    .push((remote.clone(), email.read(), email.star()));
                self.labels.push((remote.clone(), labels));
                self.filed.push((remote, role));
                Some(role)
            }
        }
    }
}

impl JmapEngine {
    /// The mailbox list, fetched again only when `Mailbox/changes` says it changed.
    pub(super) async fn refresh_mailboxes(
        &mut self,
        now: DateTime<Utc>,
    ) -> Result<Mailboxes, RuntimeError> {
        let client = self.client().await?.clone();
        let account = client.session.account.clone();
        if let Some(cached) = &self.mailboxes {
            let responses = client
                .call(
                    &[CORE, MAIL],
                    &[jmap::mailbox_changes(&account, &cached.state, "m")],
                )
                .await?;
            // Any trouble asking is answered by listing them again, which is always right.
            if let Ok(args) = responses.answer("m", "Mailbox/changes")
                && Changes::parse(args).is_ok_and(|c| c.is_empty())
            {
                return Ok(cached.clone());
            }
        }
        let responses = client
            .call(&[CORE, MAIL], &[jmap::mailbox_get(&account, "m")])
            .await?;
        let mailboxes = Mailboxes::parse(responses.answer("m", "Mailbox/get")?)?;
        self.store
            .put_folders(self.account, mailboxes.folders(self.account))?;
        self.store
            .put_caps(self.account, &self.caps(&mailboxes, now), now)?;
        self.mailboxes = Some(mailboxes.clone());
        Ok(mailboxes)
    }

    /// What changed, what is missing, and the headers of what is new.
    pub(super) async fn sync(
        &mut self,
        now: DateTime<Utc>,
        budget: usize,
        report: &mut SyncReport,
    ) -> Result<(), RuntimeError> {
        let mailboxes = self.refresh_mailboxes(now).await?;
        let client = self.client().await?.clone();
        let unfollowed = mailboxes.unfollowed();

        let mut wanted: Vec<String> = Vec::new();
        let mut state = None;
        let mut survey = true;
        if let Some((since, _)) = self.cursor()?
            && let Some((reached, created)) = self.changes(&client, &mailboxes, since).await?
        {
            state = Some(reached);
            wanted.extend(created);
            survey = false;
        }

        let held: HashSet<String> = email_ids(&self.store.remote_refs(&self.mailbox())?)
            .into_iter()
            .collect();
        // Caught up only if the server holds exactly what is held here plus what the changes
        // just named. Anything else — a backfill cut short by the budget, a message that would
        // not parse — is settled by a listing, which costs a few kilobytes of ids per thousand
        // emails.
        if !survey {
            let total = self.total(&client, &unfollowed).await?;
            let new: HashSet<&String> = wanted.iter().filter(|id| !held.contains(*id)).collect();
            survey = total != (held.len() + new.len()) as u64;
        }
        if survey {
            let (listed, listed_at) = self.survey(&client, &unfollowed, state.is_none()).await?;
            let on_server: HashSet<&String> = listed.iter().collect();
            let gone: Vec<RemoteRef> = held
                .iter()
                .filter(|id| !on_server.contains(id))
                .map(|id| RemoteRef::Jmap {
                    email_id: id.clone(),
                })
                .collect();
            self.write(Truth {
                gone,
                ..Truth::default()
            })?;
            wanted.extend(listed);
            state = state.or(listed_at);
        }

        let mut seen = HashSet::new();
        wanted.retain(|id| !held.contains(id) && seen.insert(id.clone()));
        wanted.truncate(budget);
        self.fetch_headers(&client, &mailboxes, wanted, now, report)
            .await?;

        if let Some(email_state) = state {
            self.store.ingest(
                self.account,
                Ingest {
                    mailbox: self.mailbox(),
                    validity: UidValidity::Same,
                    cursor: Some(SyncCursor::Jmap {
                        email_state,
                        mailbox_state: mailboxes.state.clone(),
                    }),
                    messages: Vec::new(),
                    flags: Vec::new(),
                    labels: Vec::new(),
                    label_names: Vec::new(),
                    gone: Vec::new(),
                },
            )?;
        }
        Ok(())
    }

    /// Apply `Email/changes` since `since`: returns the state reached and the ids to fetch, or
    /// `None` when the server can no longer say what changed since then.
    async fn changes(
        &mut self,
        client: &Client,
        mailboxes: &Mailboxes,
        since: String,
    ) -> Result<Option<(String, Vec<String>)>, RuntimeError> {
        let account = client.session.account.as_str();
        let mut since = since;
        let mut wanted = Vec::new();
        for _ in 0..CHANGE_ROUNDS {
            let calls = [
                jmap::email_changes(account, &since, CHANGES_PER_CALL, "c"),
                // What the changed emails are now, in the same round trip (RFC 8620 §3.7).
                jmap::email_get(
                    account,
                    Ids::ResultOf {
                        call: "c".to_owned(),
                        name: "Email/changes",
                        path: "/updated",
                    },
                    &["id", "mailboxIds", "keywords"],
                    "u",
                ),
            ];
            let responses = client.call(&[CORE, MAIL], &calls).await?;
            let changes = match responses.answer("c", "Email/changes") {
                Err(MethodError::CannotCalculateChanges) => return Ok(None),
                Err(e) => return Err(ProtoError::from(e).into()),
                Ok(args) => Changes::parse(args)?,
            };
            let updated = EmailSummary::parse_list(responses.answer("u", "Email/get")?)?;
            let mut truth = Truth::default();
            for email in &updated {
                // Filed and not held — moved out of Drafts, or beyond an earlier budget — is
                // wanted; `sync` drops what is already held.
                if truth.saw(email, mailboxes).is_some() {
                    wanted.push(email.id.clone());
                }
            }
            truth
                .gone
                .extend(changes.destroyed.iter().map(|id| RemoteRef::Jmap {
                    email_id: id.clone(),
                }));
            self.write(truth)?;
            wanted.extend(changes.created);
            since = changes.new_state;
            if changes.more == More::No {
                break;
            }
        }
        Ok(Some((since, wanted)))
    }

    /// How many followed emails the server holds.
    async fn total(&self, client: &Client, unfollowed: &[String]) -> Result<u64, RuntimeError> {
        let account = client.session.account.as_str();
        let responses = client
            .call(
                &[CORE, MAIL],
                &[jmap::total_query(account, unfollowed, "t")],
            )
            .await?;
        Ok(QueryPage::parse(responses.answer("t", "Email/query")?)?
            .total
            .unwrap_or(0))
    }

    /// Every followed email's id, newest first, and — when asked — the email state as of just
    /// before the listing, which is where the next `Email/changes` starts.
    async fn survey(
        &self,
        client: &Client,
        unfollowed: &[String],
        with_state: bool,
    ) -> Result<(Vec<String>, Option<String>), RuntimeError> {
        let account = client.session.account.as_str();
        let mut listed = Vec::new();
        let mut state = None;
        let mut position = 0;
        loop {
            let mut calls = Vec::new();
            let first = position == 0 && with_state;
            if first {
                // Before the query, in the same request: anything that changes after this
                // state is reported by the next pass's changes, even if the listing saw it too.
                calls.push(jmap::email_get(
                    account,
                    Ids::Listed(Vec::new()),
                    &["id"],
                    "s",
                ));
            }
            calls.push(jmap::email_query(account, unfollowed, position, PAGE, "q"));
            let responses = client.call(&[CORE, MAIL], &calls).await?;
            if first {
                state = Some(jmap::state_of(responses.answer("s", "Email/get")?)?);
            }
            let page = QueryPage::parse(responses.answer("q", "Email/query")?)?;
            let count = page.ids.len() as u64;
            listed.extend(page.ids);
            position = page.position + count;
            if count == 0 || page.total.is_some_and(|total| position >= total) {
                break;
            }
        }
        Ok((listed, state))
    }

    /// Fetch and store the headers of `ids`, with their flags, labels and filing.
    async fn fetch_headers(
        &mut self,
        client: &Client,
        mailboxes: &Mailboxes,
        ids: Vec<String>,
        now: DateTime<Utc>,
        report: &mut SyncReport,
    ) -> Result<(), RuntimeError> {
        let account = client.session.account.as_str();
        for chunk in ids.chunks(client.session.limits.max_objects_in_get) {
            let responses = client
                .call(
                    &[CORE, MAIL],
                    &[jmap::email_get(
                        account,
                        Ids::Listed(chunk.to_vec()),
                        jmap::SUMMARY,
                        "h",
                    )],
                )
                .await?;
            let emails = EmailSummary::parse_list(responses.answer("h", "Email/get")?)?;
            let mut truth = Truth::default();
            let mut by_role: Vec<(MailboxRole, Vec<Arrival>, bool)> = Vec::new();
            for email in &emails {
                let Some(role) = truth.saw(email, mailboxes) else {
                    continue;
                };
                // The header block where the server gave one; the whole message where it did
                // not, which costs more and is always right.
                let (raw, headers_only) = if email.headers.is_empty() {
                    (client.download(&email.blob_id).await?, false)
                } else {
                    (email.raw_headers(), true)
                };
                let arrival = Arrival {
                    remote: RemoteRef::Jmap {
                        email_id: email.id.clone(),
                    },
                    raw,
                };
                match by_role
                    .iter_mut()
                    .find(|(r, _, h)| *r == role && *h == headers_only)
                {
                    Some((_, batch, _)) => batch.push(arrival),
                    None => by_role.push((role, vec![arrival], headers_only)),
                }
            }
            for (role, arrivals, headers_only) in by_role {
                report.headers_fetched += arrivals.len();
                let stored = absorb_into(
                    &self.store,
                    self.account,
                    Destination {
                        mailbox: self.mailbox(),
                        role,
                    },
                    None,
                    arrivals,
                    headers_only,
                    now,
                )?;
                report.arrived.extend(crate::engine::first_stored(&stored));
            }
            // Flags and labels after the messages they sit on exist.
            truth.gone.clear();
            self.write(truth)?;
        }
        Ok(())
    }

    /// Download and store the bodies of the newest messages held without one.
    pub(super) async fn fetch_bodies(
        &mut self,
        now: DateTime<Utc>,
        budget: usize,
        report: &mut SyncReport,
    ) -> Result<(), RuntimeError> {
        let wanted = email_ids(&self.store.unfetched_in(&self.mailbox(), budget as u32)?);
        if wanted.is_empty() {
            return Ok(());
        }
        let client = self.client().await?.clone();
        let account = client.session.account.as_str();
        for chunk in wanted.chunks(client.session.limits.max_objects_in_get) {
            let responses = client
                .call(
                    &[CORE, MAIL],
                    &[jmap::blob_ids(account, chunk.to_vec(), "b")],
                )
                .await?;
            let emails = EmailSummary::parse_list(responses.answer("b", "Email/get")?)?;
            let mut arrivals = Vec::new();
            for email in emails {
                arrivals.push(Arrival {
                    raw: client.download(&email.blob_id).await?,
                    remote: RemoteRef::Jmap { email_id: email.id },
                });
            }
            report.bodies_fetched += arrivals.len();
            absorb_into(
                &self.store,
                self.account,
                // Every one of these is already held and keeps its role: a body arriving says
                // nothing about where a message is filed. `Archive` is the one role the store
                // never refiles an arrival for (`filing::after_arrival` moves only into the
                // inbox), so naming it here cannot move anything.
                Destination {
                    mailbox: self.mailbox(),
                    role: MailboxRole::Archive,
                },
                None,
                arrivals,
                false,
                now,
            )?;
        }
        Ok(())
    }

    /// Write what the server said about emails already held.
    fn write(&self, truth: Truth) -> Result<(), RuntimeError> {
        let Truth {
            flags,
            labels,
            filed,
            gone,
        } = truth;
        if flags.is_empty() && labels.is_empty() && gone.is_empty() && filed.is_empty() {
            return Ok(());
        }
        self.store.ingest(
            self.account,
            Ingest {
                mailbox: self.mailbox(),
                validity: UidValidity::Same,
                cursor: None,
                messages: Vec::new(),
                flags,
                labels: Vec::new(),
                label_names: labels,
                gone,
            },
        )?;
        self.store.refile(self.account, &filed)?;
        Ok(())
    }
}
