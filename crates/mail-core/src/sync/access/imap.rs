//! IMAP: a socket per session, signed in with a password or an OAuth token.

use super::{Access, Answer, Pass, Wait, verdict};
use crate::error::CoreError;
use crate::error::Logged;
use crate::sync::live::{Lost, extra_idle, pushes, unsupported};
use crate::sync::report::{Done, Failure};
use crate::sync::run::{Run, Signed, signed_in};
use crate::sync::{Mode, fetch_folder, imap_engine, poll_floor, pushed_folders, to_sync};
use mail_domain::{Filter, LabelId, MailboxRef, MessageId};
use mail_proto::Storage;
use mail_proto::backend::ImapBackend;
use mail_runtime::{AccountEngine, Cancel, Notify, RuntimeError, Searched, Woke};
use std::time::Duration;
use tokio::sync::watch;

/// How long extra connections have to end their IDLE in protocol once the wait is over.
const WIND_DOWN: Duration = Duration::from_secs(3);

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

    fn wait<'a>(&'a self, run: &'a Run<'a>, _: &'a str) -> Answer<'a, Box<dyn Wait>, Lost> {
        Box::pin(async move {
            let account = run.account;
            if !pushes(account) {
                return Err(unsupported("this account has no push to wait on"));
            }
            let Signed { held, renewal } = signed_in(run, Mode::Watch).await?;
            let folders = {
                use mail_store::Store as _;
                run.store
                    .folders(account.id.clone())
                    .or_log_default("the folders could not be read to watch them")
            };
            let others = pushed_folders(account, &folders);
            let build = |held: mail_secrets::Held| {
                let engine = imap_engine(run.store, account, held, run.secrets.clone());
                match &renewal {
                    Some(renewal) => engine.with_tokens(renewal.clone()),
                    None => engine,
                }
            };
            let extras = others
                .iter()
                .take(extra_idle(&account.caps))
                .map(|m| (m.clone(), build(held.clone())))
                .collect();
            let engine = build(held).with_also(others);
            let inbox = to_sync(account, &[])
                .into_iter()
                .next()
                .ok_or_else(|| unsupported("nothing to watch"))?;
            let waiting: Box<dyn Wait> = Box::new(Idling {
                engine: Box::new(engine),
                inbox,
                floor: poll_floor(&account.caps.watch),
                extras,
            });
            Ok(waiting)
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

/// An IMAP connection kept open on the inbox.
struct Idling {
    engine: Box<AccountEngine<ImapBackend>>,
    inbox: MailboxRef,
    floor: Duration,
    /// Engines for folders besides the inbox, used only where the server has no `NOTIFY` for the
    /// main one to hear of them through. Opened lazily: building one connects to nothing.
    extras: Vec<(MailboxRef, AccountEngine<ImapBackend>)>,
}

impl Wait for Idling {
    fn wait<'a>(&'a mut self, cancel: &'a mut Cancel) -> Answer<'a, Woke, RuntimeError> {
        Box::pin(async move {
            let Idling {
                engine,
                inbox,
                floor,
                extras,
            } = self;
            // Through `NOTIFY` where the server has it, the one connection hears of every
            // folder. Without it, one connection each, up to the limit.
            if extras.is_empty() || engine.notifies() != Notify::Absent {
                return engine.wait(inbox, cancel, *floor).await;
            }
            wait_on_each(engine, inbox, *floor, extras, cancel).await
        })
    }
}

/// [`AccountEngine::wait`] on the inbox, with every extra folder idled on a connection of its
/// own meanwhile: whichever has news ends the wait as mail.
///
/// An extra that fails, or whose server ends its IDLE without news, drops out of this wait and
/// is tried again on the next: a folder another client deleted must not end the account's push.
async fn wait_on_each(
    engine: &mut AccountEngine<ImapBackend>,
    inbox: &MailboxRef,
    floor: Duration,
    extras: &mut [(MailboxRef, AccountEngine<ImapBackend>)],
    cancel: &mut Cancel,
) -> Result<Woke, RuntimeError> {
    use futures_util::StreamExt;
    use futures_util::stream::FuturesUnordered;

    let (stop, stopped) = watch::channel(false);
    let mut others: FuturesUnordered<_> = extras
        .iter_mut()
        .map(|(mailbox, engine)| {
            let mut stopped = stopped.clone();
            async move { engine.watch(mailbox, &mut stopped).await }
        })
        .collect();
    let primary = engine.wait(inbox, cancel, floor);
    tokio::pin!(primary);

    let woke = loop {
        tokio::select! {
            woke = &mut primary => break woke,
            Some(heard) = others.next() => match heard {
                Ok(true) => break Ok(Woke::Mail),
                Ok(false) => {}
                Err(e) => log::warn!("a folder could not be watched: {e}"),
            },
        }
    };
    // Ended in protocol, so each connection stays usable; bounded, so a dead one cannot hold
    // the wait.
    let _ = stop.send(true);
    let drained = async { while others.next().await.is_some() {} };
    if tokio::time::timeout(WIND_DOWN, drained).await.is_err() {
        log::debug!("an extra connection did not end its IDLE in time");
    }
    woke
}
