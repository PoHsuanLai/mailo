//! JMAP: HTTPS to a session URL, one request round for the whole account.

use super::{Access, Answer, Pass};
use crate::sync::report::{Done, Failure};
use crate::sync::run::{Run, signed_in};
use crate::sync::Configured;
use mail_domain::{Filter, Folder, JMAP_ALL, LabelId, MailboxRef};
use mail_proto::Storage;
use mail_runtime::{JmapEngine, Searched};

/// JMAP.
pub(crate) struct Jmap;

/// An engine for `run`'s account.
///
/// It reads its credential itself, so it asks for nothing signed in: that is [`Jmap::pass`]'s.
pub(super) fn engine(run: &Run<'_>) -> Result<JmapEngine, Failure> {
    JmapEngine::new(
        run.account.id.clone(),
        run.account.plan.clone(),
        run.store.clone(),
        run.secrets.clone(),
    )
    .map_err(|e| Failure::of("", &e))
}

impl Access for Jmap {
    fn pass<'a>(&'a self, run: &'a Run<'a>, pass: Pass<'a>) -> Answer<'a, Done> {
        Box::pin(async move {
            // For its side effect: an expired token is renewed and kept where the engine reads it.
            signed_in(run, pass.mode).await?;
            let mut engine = engine(run)?;
            crate::sync::jmap::drive(&mut engine, run.account, pass.cancel, run.now, pass.emit)
                .await
        })
    }

    fn search<'a>(
        &'a self,
        run: &'a Run<'a>,
        filter: &'a Filter,
        labels: &'a [(LabelId, String)],
    ) -> Answer<'a, Searched> {
        Box::pin(async move {
            engine(run)?
                .search_jmap(filter, labels, run.now)
                .await
                .map_err(|e| Failure::of("the server was not searched: ", &e))
        })
    }

    fn quota<'a>(&'a self, run: &'a Run<'a>) -> Answer<'a, Option<Storage>> {
        Box::pin(async move {
            engine(run)?
                .quota_jmap()
                .await
                .map_err(|e| Failure::of("the server did not say how much storage is used: ", &e))
        })
    }

    /// One mailbox, the whole account: JMAP reports changes across every mailbox at once.
    fn mailboxes(&self, account: &Configured, _: &[Folder]) -> Vec<MailboxRef> {
        vec![MailboxRef {
            account: account.id.clone(),
            path: JMAP_ALL.to_owned(),
        }]
    }
}
