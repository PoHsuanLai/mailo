//! IMAP: a socket per session, signed in with a password or an OAuth token.

use super::{Access, Answer, Pass, verdict};
use crate::error::CoreError;
use crate::sync::report::{Done, Failure};
use crate::sync::run::{Run, Signed, signed_in};
use crate::sync::{Mode, fetch_folder, imap_engine};
use mail_domain::{Filter, LabelId, MessageId};
use mail_proto::Storage;
use mail_proto::backend::ImapBackend;
use mail_runtime::{AccountEngine, Searched};
use tokio::sync::watch;

/// IMAP.
pub(crate) struct Imap;

/// An engine for `run`'s account, signed in with a fresh credential and kept fresh.
pub(crate) async fn signed_engine(
    run: &Run<'_>,
    mode: Mode,
) -> Result<AccountEngine<ImapBackend>, Failure> {
    let Signed { held, renewal } = signed_in(run, mode).await?;
    let engine = imap_engine(run.store, run.account, held, run.secrets.clone());
    Ok(match renewal {
        Some(source) => engine.with_tokens(source),
        None => engine,
    })
}

impl Access for Imap {
    fn pass<'a>(&'a self, run: &'a Run<'a>, pass: Pass<'a>) -> Answer<'a, Done> {
        Box::pin(async move {
            let mut engine = signed_engine(run, pass.mode)
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
            let mut engine = signed_engine(run, Mode::Once).await?;
            engine
                .fetch_body(message, &mut cancel, run.now)
                .await
                .map_err(|e| Failure::of("cannot download the message: ", &e))
        })
    }

    fn folder_now<'a>(&'a self, run: &'a Run<'a>, path: &'a str) -> Answer<'a, Done> {
        Box::pin(async move {
            let mut engine = signed_engine(run, Mode::Once).await?;
            engine.reachable().await.map_err(|e| Failure::of("", &e))?;
            fetch_folder(&mut engine, run, path).await
        })
    }

    fn search<'a>(
        &'a self,
        run: &'a Run<'a>,
        filter: &'a Filter,
        labels: &'a [(LabelId, String)],
    ) -> Answer<'a, Searched> {
        Box::pin(async move {
            let (_tx, mut cancel) = watch::channel(false);
            let mut engine = signed_engine(run, Mode::Once).await?;
            engine
                .search_imap(filter, labels, &mut cancel, run.now)
                .await
                .map_err(|e| Failure::of("the server was not searched: ", &e))
        })
    }

    fn quota<'a>(&'a self, run: &'a Run<'a>) -> Answer<'a, Option<Storage>> {
        Box::pin(async move {
            let (_tx, mut cancel) = watch::channel(false);
            let mut engine = signed_engine(run, Mode::Once).await?;
            engine
                .quota_imap(&mut cancel)
                .await
                .map_err(|e| Failure::of("the server did not say how much storage is used: ", &e))
        })
    }

    fn check_sign_in<'a>(&'a self, run: &'a Run<'a>) -> Answer<'a, (), CoreError> {
        Box::pin(async move {
            let (_tx, mut cancel) = watch::channel(false);
            let mut engine = signed_engine(run, Mode::Once).await.map_err(crate::sync::pass_error)?;
            let asked = engine.refresh_caps(&mut cancel, run.now).await;
            verdict(run, asked)
        })
    }
}
