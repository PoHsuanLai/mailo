//! JMAP: HTTPS to a session URL, one request round for the whole account.

use super::{Access, Answer, Pass, Wait};
use crate::sync::live::{Lost, lost, pushes, unsupported};
use crate::sync::report::{Done, Failure};
use crate::sync::run::{Run, signed_in};
use crate::sync::{Configured, Mode, poll_floor};
use mail_domain::{Filter, Folder, JMAP_ALL, LabelId, MailboxRef};
use mail_proto::Storage;
use mail_runtime::{Cancel, JmapEngine, RuntimeError, Searched, Woke};
use std::time::Duration;

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

    fn wait<'a>(&'a self, run: &'a Run<'a>, _: &'a str) -> Answer<'a, Box<dyn Wait>, Lost> {
        Box::pin(async move {
            if !pushes(run.account) {
                return Err(unsupported("this account has no push to wait on"));
            }
            // For its side effect, as a pass's is.
            signed_in(run, Mode::Watch).await?;
            let mut engine = engine(run)?;
            engine.connect().await.map_err(|e| lost(&e))?;
            // The stored capabilities said push; the server's own session is the authority.
            if !engine.pushes() {
                return Err(unsupported("the server offers no event source"));
            }
            let waiting: Box<dyn Wait> = Box::new(Pushing {
                engine: Box::new(engine),
                floor: poll_floor(&run.account.caps.watch),
            });
            Ok(waiting)
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

/// A JMAP session's event source.
struct Pushing {
    engine: Box<JmapEngine>,
    floor: Duration,
}

impl Wait for Pushing {
    fn wait<'a>(&'a mut self, cancel: &'a mut Cancel) -> Answer<'a, Woke, RuntimeError> {
        Box::pin(self.engine.wait(cancel, self.floor))
    }
}
