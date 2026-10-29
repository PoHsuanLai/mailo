//! Microsoft Graph: HTTPS to a fixed host, with a Graph token beside the sign-in's own.

use super::{Access, Answer, Pass};
use crate::sync::report::{Done, Failure};
use crate::sync::run::{Run, Signed, signed_in};
use crate::sync::{Mode, fetch_folder, graph_engine, sending_token};
use mail_domain::{Filter, LabelId, MessageId};
use mail_runtime::graph::read::OverHttp;
use mail_runtime::{AccountEngine, Searched};
use tokio::sync::watch;

/// Microsoft Graph.
pub(crate) struct Graph;

/// An engine for `run`'s account, signed in. The Graph token it reads with is the caller's to
/// have made sure of ([`sending_token`]).
async fn built(run: &Run<'_>, mode: Mode) -> Result<AccountEngine<OverHttp>, Failure> {
    let Signed { renewal, .. } = signed_in(run, mode).await?;
    let engine = graph_engine(run.store, run.account, run.secrets.clone())
        .map_err(|e| Failure::fatal(e.to_string()))?;
    Ok(match renewal {
        Some(source) => engine.with_tokens(source),
        None => engine,
    })
}

/// [`built`], after making sure of a Graph token valid now.
async fn signed_engine(run: &Run<'_>) -> Result<AccountEngine<OverHttp>, Failure> {
    sending_token(run.account, run.secrets.as_ref(), run.registry, run.now).await?;
    built(run, Mode::Once).await
}

impl Access for Graph {
    fn pass<'a>(&'a self, run: &'a Run<'a>, pass: Pass<'a>) -> Answer<'a, Done> {
        Box::pin(async move {
            let mut engine = built(run, pass.mode)
                .await?
                .downloading_since(run.account.window.since(run.now));
            crate::sync::pass(
                &mut engine,
                &pass.mailboxes,
                run.account,
                pass.cancel,
                run.now,
                pass.emit,
            )
            .await
        })
    }

    fn body<'a>(&'a self, run: &'a Run<'a>, message: MessageId) -> Answer<'a, ()> {
        Box::pin(async move {
            let (_tx, mut cancel) = watch::channel(false);
            let mut engine = signed_engine(run).await?;
            engine
                .fetch_body(message, &mut cancel, run.now)
                .await
                .map_err(|e| Failure::of("cannot download the message: ", &e))
        })
    }

    fn folder_now<'a>(&'a self, run: &'a Run<'a>, path: &'a str) -> Answer<'a, Done> {
        Box::pin(async move {
            let mut engine = signed_engine(run).await?;
            fetch_folder(&mut engine, run, path).await
        })
    }

    fn search<'a>(
        &'a self,
        run: &'a Run<'a>,
        filter: &'a Filter,
        _: &'a [(LabelId, String)],
    ) -> Answer<'a, Searched> {
        Box::pin(async move {
            signed_engine(run)
                .await?
                .search_graph(filter, run.now)
                .await
                .map_err(|e| Failure::of("the server was not searched: ", &e))
        })
    }
}
