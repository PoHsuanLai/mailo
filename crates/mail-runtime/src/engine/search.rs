//! Searching an IMAP account's server, or a Microsoft Graph account's, from its engine.
//!
//! Both end the same way: the addresses the server named, newest first and at most
//! [`SERVER_HITS`]; those not held here fetched as headers and absorbed by
//! [`AccountEngine::absorb_headers`], the path a sync keeps headers by; the new ones marked as
//! found on the server.

use super::{AccountEngine, first_stored};
use crate::renewal::{AfterRefusal, Token};
use crate::search::{SERVER_HITS, Searched, ServerHits};
use crate::{Cancel, RuntimeError, drive};
use chrono::{DateTime, Utc};
use mail_domain::{
    Filter, LabelId, MailboxRef, ProtoOp, ReadState, RemoteRef, Retry, Retryable, Star,
};
use mail_proto::backend::{Found, ImapBackend};
use mail_proto::search::imap::{ImapCtx, ImapPlan};
use mail_proto::search::{Asked, graph, imap};
use mail_proto::{Backend, ProtoOutcome};
use mail_store::Store;

impl AccountEngine<ImapBackend> {
    /// Search the IMAP server for `filter`, and keep what it finds here.
    ///
    /// `labels` is every label this client knows on the account, by name, for `label:` on
    /// Gmail. [`Searched::Unsaid`], with nothing sent, when the query cannot be asked faithfully.
    pub async fn search_imap(
        &mut self,
        filter: &Filter,
        labels: &[(LabelId, String)],
        cancel: &mut Cancel,
        now: DateTime<Utc>,
    ) -> Result<Searched, RuntimeError> {
        let folders = self.store.folders(self.account.clone())?;
        let caps = self.caps().clone();
        let ctx = ImapCtx {
            account: self.account.clone(),
            folders: &folders,
            roles: &caps.folders,
            labels: caps.labels,
            labels_named: labels,
        };
        let plan = match imap::translate(filter, &ctx) {
            Err(unsaid) => return Ok(Searched::Unsaid(unsaid)),
            Ok(Asked::Nothing) => return Ok(Searched::Found(ServerHits::default())),
            Ok(Asked::Ask(plan)) => plan,
        };
        let found = self.imap_search(plan, cancel).await?;
        let mut hits = ServerHits::default();
        let mut room = SERVER_HITS;
        for Found {
            mailbox,
            uidvalidity,
            uids,
            count,
        } in found
        {
            // UIDs count up as mail arrives: the highest are the newest.
            let remotes: Vec<RemoteRef> = uids
                .iter()
                .rev()
                .take(room)
                .map(|&uid| RemoteRef::Imap {
                    mailbox: mailbox.clone(),
                    uidvalidity,
                    uid,
                })
                .collect();
            room -= remotes.len();
            hits.more += count.saturating_sub(remotes.len() as u64);
            let mailbox = MailboxRef {
                account: self.account.clone(),
                path: mailbox,
            };
            let wanted = self.not_held(&remotes)?;
            if !wanted.is_empty() {
                let outcome = self
                    .run(ProtoOp::FetchHeaders { remotes: wanted }, cancel)
                    .await?;
                if let ProtoOutcome::Fetched { items, flags } = outcome {
                    self.keep_found(&mailbox, items, flags, now, &mut hits)?;
                }
            }
            hits.hold(self.held(&remotes)?);
        }
        Ok(Searched::Found(hits))
    }

    /// Run the search walk on a connection of its own, with a fresh token, and once more with a
    /// new one if the server refuses it — as every operation is run ([`Self::run`]).
    async fn imap_search(
        &mut self,
        plan: ImapPlan,
        cancel: &mut Cancel,
    ) -> Result<Vec<Found>, RuntimeError> {
        if let Some(renewal) = &self.renewal {
            renewal.ahead(Token::Incoming).await?;
        }
        let first = self.imap_search_once(plan.clone(), cancel).await;
        let refused = matches!(&first, Err(e) if matches!(e.retry(), Retry::NeedsReauth));
        let Some(renewal) = self.renewal.as_ref().filter(|_| refused) else {
            return first;
        };
        match renewal.after_refusal(Token::Incoming).await? {
            AfterRefusal::TryAgain => self.imap_search_once(plan, cancel).await,
            AfterRefusal::StillRefused => first,
        }
    }

    async fn imap_search_once(
        &mut self,
        plan: ImapPlan,
        cancel: &mut Cancel,
    ) -> Result<Vec<Found>, RuntimeError> {
        let mut transport = self.connect().await?;
        let mut walk = self.backend.searching(plan);
        drive(&mut walk, &mut transport, cancel).await
    }
}

impl<B: Backend> AccountEngine<B> {
    /// Search Microsoft Graph for `filter`, and keep what it finds here. For an engine that
    /// reads through Graph ([`Self::with_graph_reader`]); any other is refused.
    ///
    /// The headers are written from the search's own answer, as a delta's are, so nothing but
    /// the search itself is asked of Graph.
    pub async fn search_graph(
        &mut self,
        filter: &Filter,
        now: DateTime<Utc>,
    ) -> Result<Searched, RuntimeError> {
        let plan = match graph::translate(filter, self.account.clone()) {
            Err(unsaid) => return Ok(Searched::Unsaid(unsaid)),
            Ok(Asked::Nothing) => return Ok(Searched::Found(ServerHits::default())),
            Ok(Asked::Ask(plan)) => plan,
        };
        if let Some(renewal) = &self.renewal {
            renewal.ahead(Token::Incoming).await?;
        }
        let token = match self
            .secret(porter_core::SecretPurpose::OutgoingPassword)
            .await
        {
            Ok(porter_core::Credential::OAuth { access, .. }) => access,
            _ => {
                return Err(RuntimeError::Secrets(format!(
                    "no Microsoft Graph sign-in is stored for {}",
                    self.plan.address
                )));
            }
        };
        let Some(reader) = self.graph.as_mut() else {
            return Err(RuntimeError::UnsupportedIo(
                "this account does not read through Microsoft Graph".to_owned(),
            ));
        };
        let found = reader.search(&plan, token.expose(), SERVER_HITS).await?;
        let mut hits = ServerHits {
            more: u64::from(found.more),
            ..ServerHits::default()
        };
        // Grouped by folder, in the order Graph gave them: each folder is a mailbox here.
        let mut folders: Vec<String> = Vec::new();
        for hit in &found.hits {
            if let RemoteRef::Graph { mailbox, .. } = &hit.remote
                && !folders.contains(mailbox)
            {
                folders.push(mailbox.clone());
            }
        }
        for path in folders {
            let mailbox = MailboxRef {
                account: self.account.clone(),
                path: path.clone(),
            };
            let here: Vec<_> = found
                .hits
                .iter()
                .filter(
                    |h| matches!(&h.remote, RemoteRef::Graph { mailbox, .. } if *mailbox == path),
                )
                .collect();
            let remotes: Vec<RemoteRef> = here.iter().map(|h| h.remote.clone()).collect();
            let wanted = self.not_held(&remotes)?;
            let items = here
                .iter()
                .filter(|h| wanted.contains(&h.remote))
                .map(|h| (h.remote.clone(), h.headers.clone()))
                .collect::<Vec<_>>();
            let flags = here
                .iter()
                .filter(|h| wanted.contains(&h.remote))
                .map(|h| (h.remote.clone(), h.read, h.star))
                .collect();
            if !items.is_empty() {
                self.keep_found(&mailbox, items, flags, now, &mut hits)?;
            }
        }
        let every: Vec<RemoteRef> = found.hits.iter().map(|h| h.remote.clone()).collect();
        hits.hold(self.held(&every)?);
        Ok(Searched::Found(hits))
    }

    /// Of `remotes`, those that name nothing held here.
    fn not_held(&self, remotes: &[RemoteRef]) -> Result<Vec<RemoteRef>, RuntimeError> {
        let held = self.store.held_at(self.account.clone(), remotes)?;
        Ok(remotes
            .iter()
            .filter(|r| !held.iter().any(|(h, _)| h == *r))
            .cloned()
            .collect())
    }

    /// The messages held at `remotes`, in their order.
    fn held(&self, remotes: &[RemoteRef]) -> Result<Vec<mail_domain::MessageId>, RuntimeError> {
        Ok(self
            .store
            .held_at(self.account.clone(), remotes)?
            .into_iter()
            .map(|(_, id)| id)
            .collect())
    }

    /// Keep headers a search fetched, and mark what is new to this computer as found there.
    fn keep_found(
        &self,
        mailbox: &MailboxRef,
        items: Vec<(RemoteRef, Vec<u8>)>,
        flags: Vec<(RemoteRef, ReadState, Star)>,
        now: DateTime<Utc>,
        hits: &mut ServerHits,
    ) -> Result<(), RuntimeError> {
        let stored = self.absorb_headers(mailbox, items, flags, now)?;
        let new: Vec<_> = first_stored(&stored).collect();
        self.store.mark_found(self.account.clone(), &new, now)?;
        hits.fetched += new.len();
        Ok(())
    }
}
