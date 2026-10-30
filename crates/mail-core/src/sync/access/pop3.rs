//! POP3: one mailbox, downloaded, with the server's copies removed afterwards where the account
//! says so.

use super::{Access, Answer, Pass, verdict};
use crate::error::CoreError;
use crate::sync::report::{Done, Failure};
use crate::sync::run::{Run, Signed, signed_in};
use crate::sync::{Mode, Removing, pop3_engine, remove_copies};
use mail_engine::AccountEngine;
use mail_proto::backend::Pop3Backend;
use tokio::sync::watch;

/// POP3.
pub(crate) struct Pop3;

/// An engine for `run`'s account, signed in with its password.
async fn signed_engine(run: &Run<'_>, mode: Mode) -> Result<AccountEngine<Pop3Backend>, Failure> {
    let Signed { held, renewal } = signed_in(run, mode).await?;
    let engine = pop3_engine(run.store, run.account, &held, run.secrets.clone())?;
    Ok(match renewal {
        Some(source) => engine.with_tokens(source),
        None => engine,
    })
}

impl Access for Pop3 {
    fn pass<'a>(&'a self, run: &'a Run<'a>, pass: Pass<'a>) -> Answer<'a, Done> {
        Box::pin(async move {
            let mut engine = signed_engine(run, pass.mode)
                .await?
                .downloading_since(run.account.window.since(run.now));
            let mut done = crate::sync::pass(
                &mut engine,
                &pass.mailboxes,
                run.account,
                pass.cancel,
                run.now,
                pass.emit,
            )
            .await?;
            let removing = Removing {
                store: run.store,
                account: run.account,
                now: run.now,
            };
            remove_copies(&mut engine, &removing, pass.cancel, &mut done).await;
            Ok::<Done, Failure>(done)
        })
    }

    fn check_sign_in<'a>(&'a self, run: &'a Run<'a>) -> Answer<'a, (), CoreError> {
        Box::pin(async move {
            let (_tx, mut cancel) = watch::channel(false);
            let mut engine = signed_engine(run, Mode::Once)
                .await
                .map_err(crate::sync::pass_error)?;
            let asked = engine.refresh_caps(&mut cancel, run.now).await;
            verdict(run, asked)
        })
    }
}
