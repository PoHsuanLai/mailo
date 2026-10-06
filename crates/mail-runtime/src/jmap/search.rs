//! Searching a JMAP account's server: `Email/query` with the query as a filter (RFC 8621 §4.4),
//! the newest [`SERVER_HITS`] of what it finds, and the headers of those not held here fetched
//! as a sync fetches them — with one difference: an email whose server gives no header fields is
//! left on the server rather than downloaded whole, since a search never fetches a body.

use super::{JmapEngine, Whole, email_ids};
use crate::search::{SERVER_HITS, Searched, ServerHits};
use crate::{RuntimeError, SyncReport};
use chrono::{DateTime, Utc};
use mail_domain::{Filter, LabelId, RemoteRef};
use mail_proto::jmap::{CORE, MAIL, QueryPage};
use mail_proto::search::Asked;
use mail_proto::search::jmap::{JmapCtx, query, translate};
use mail_store::Store;

impl JmapEngine {
    /// Search the server for `filter`, and keep what it finds here.
    ///
    /// `labels` is every label this client knows on the account, by name: on JMAP a label is a
    /// mailbox. [`Searched::Unsaid`], with nothing sent, when the query cannot be asked
    /// faithfully.
    pub async fn search_jmap(
        &mut self,
        filter: &Filter,
        labels: &[(LabelId, String)],
        now: DateTime<Utc>,
    ) -> Result<Searched, RuntimeError> {
        let result = self.searched(filter, labels, now).await;
        if let Err(e) = &result {
            self.forget_if_stale(e);
        }
        result
    }

    async fn searched(
        &mut self,
        filter: &Filter,
        labels: &[(LabelId, String)],
        now: DateTime<Utc>,
    ) -> Result<Searched, RuntimeError> {
        let mailboxes = self.refresh_mailboxes(now).await?;
        let ctx = JmapCtx {
            account: self.account.clone(),
            mailboxes: &mailboxes,
            labels_named: labels,
        };
        let condition = match translate(filter, &ctx) {
            Err(unsaid) => return Ok(Searched::Unsaid(unsaid)),
            Ok(Asked::Nothing) => return Ok(Searched::Found(ServerHits::default())),
            Ok(Asked::Ask(condition)) => condition,
        };
        let client = self.client().await?.clone();
        let call = query(
            &client.session.account,
            condition,
            &mailboxes.unfollowed(),
            SERVER_HITS as u64,
            "q",
        );
        let responses = client.call(&[CORE, MAIL], &[call]).await?;
        let page = QueryPage::parse(responses.answer("q", "Email/query")?)?;
        let found: Vec<RemoteRef> = page
            .ids
            .iter()
            .take(SERVER_HITS)
            .map(|id| RemoteRef::Jmap {
                email_id: id.clone(),
            })
            .collect();
        let held = self.store.held_at(self.account.clone(), &found)?;
        let wanted: Vec<RemoteRef> = found
            .iter()
            .filter(|r| !held.iter().any(|(h, _)| h == *r))
            .cloned()
            .collect();
        let mut report = SyncReport::default();
        self.fetch_headers(
            &client,
            &mailboxes,
            email_ids(&wanted),
            now,
            &mut report,
            Whole::Never,
        )
        .await?;
        self.store
            .mark_found(self.account.clone(), &report.arrived, now)?;
        let mut hits = ServerHits {
            fetched: report.arrived.len(),
            more: page
                .total
                .map_or(0, |total| total.saturating_sub(found.len() as u64)),
            ..ServerHits::default()
        };
        hits.hold(
            self.store
                .held_at(self.account.clone(), &found)?
                .into_iter()
                .map(|(_, id)| id),
        );
        Ok(Searched::Found(hits))
    }
}
