//! What an account's incoming protocol *does*, as one trait with an implementor each.
//!
//! [`Incoming`] is the stored name of a protocol and stays an enum; what it can do is
//! [`Incoming::caps`], a table of plain facts. This is the other half: the thing that signs in,
//! builds the protocol's engine and drives it. The protocol is read from the stored plan at run
//! time, so it is a trait object, and [`access`] is the one place in this crate that matches on
//! [`Incoming`] to pick it.
//!
//! An operation a protocol cannot do is a method it does not override, and the default refuses
//! with one sentence that names the protocol ([`refuse`]).

mod graph;
pub(super) mod imap;
mod jmap;
mod local;
mod pop3;

use super::report::{Done, Emit, Failure};
use super::run::Run;
use super::{Configured, Mode, inbox_and_sent};
use crate::error::CoreError;
use mail_domain::{Filter, Incoming, LabelId, MailboxRef, MessageId};
use mail_proto::Storage;
use mail_runtime::{Cancel, RuntimeError, Searched};
use std::future::Future;
use std::pin::Pin;

/// What an operation on a server answers with: a boxed future, which is not `Send` because a
/// pass's progress sink is not.
pub(crate) type Answer<'a, T, E = Failure> = Pin<Box<dyn Future<Output = Result<T, E>> + 'a>>;

/// What one pass is given beyond the [`Run`]: how long its `now` is good for, the mailboxes to
/// fetch, the signal that stops it, and where it says how far it has got.
pub(crate) struct Pass<'a> {
    pub mode: Mode,
    pub mailboxes: Vec<MailboxRef>,
    pub cancel: &'a mut Cancel,
    pub emit: Emit<'a>,
}

/// One incoming protocol, driven from an account's stored plan.
///
/// Every method but [`pass`](Self::pass) is for what only some protocols do; the default says
/// the protocol cannot (or, for a quota, that there is none).
pub(crate) trait Access: Send + Sync {
    /// One pass over the account: sign in, fetch, settle the outbox.
    fn pass<'a>(&'a self, run: &'a Run<'a>, pass: Pass<'a>) -> Answer<'a, Done>;

    /// Download one message's body and store it.
    fn body<'a>(&'a self, run: &'a Run<'a>, message: MessageId) -> Answer<'a, ()> {
        let _ = message;
        refuse(run, "fetch one message on its own")
    }

    /// Fetch one folder now.
    fn folder_now<'a>(&'a self, run: &'a Run<'a>, path: &'a str) -> Answer<'a, Done> {
        let _ = path;
        refuse(run, "fetch one folder")
    }

    /// Search the server for `filter`, and keep what it finds. `labels` is the account's labels
    /// with their names, for `label:`.
    fn search<'a>(
        &'a self,
        run: &'a Run<'a>,
        filter: &'a Filter,
        labels: &'a [(LabelId, String)],
    ) -> Answer<'a, Searched> {
        let _ = (filter, labels);
        refuse(run, "search the server")
    }

    /// How much storage the server says is used. `None` where it keeps no quota.
    fn quota<'a>(&'a self, run: &'a Run<'a>) -> Answer<'a, Option<Storage>> {
        let _ = run;
        Box::pin(std::future::ready(Ok(None)))
    }

    /// Try the account's password against its server, as a pass would, and say what it
    /// answered. Only the two that sign in with one (IMAP, POP3) have anything to try.
    fn check_sign_in<'a>(&'a self, run: &'a Run<'a>) -> Answer<'a, (), CoreError> {
        let _ = run;
        Box::pin(std::future::ready(Ok(())))
    }

    /// The mailboxes a pass fetches, in the order it fetches them, given the folders the store
    /// holds.
    fn mailboxes(&self, account: &Configured, folders: &[mail_domain::Folder]) -> Vec<MailboxRef> {
        inbox_and_sent(account, folders)
    }
}

/// The protocol `incoming` names. The only match on [`Incoming`] in this crate.
pub(crate) fn access(incoming: &Incoming) -> &'static dyn Access {
    match incoming {
        Incoming::Imap { .. } => &imap::Imap,
        Incoming::Pop3 { .. } => &pop3::Pop3,
        Incoming::Graph => &graph::Graph,
        Incoming::Jmap { .. } => &jmap::Jmap,
        Incoming::Local => &local::Local,
    }
}

/// What a protocol that cannot do `what` answers, naming the account and the protocol.
pub(crate) fn refusal(run: &Run<'_>, what: &str) -> Failure {
    Failure::fatal(format!(
        "{} is {}, which cannot {what}",
        run.account.address,
        run.account.plan.incoming.caps().name
    ))
}

/// [`refusal`], as the answer of an operation.
fn refuse<'a, T: 'a, E: From<Failure> + 'a>(run: &Run<'_>, what: &str) -> Answer<'a, T, E> {
    Box::pin(std::future::ready(Err(refusal(run, what).into())))
}

/// What a server's answer to a sign-in check says, as the error the person is shown.
fn verdict(run: &Run<'_>, asked: Result<impl Sized, RuntimeError>) -> Result<(), CoreError> {
    let host = run
        .account
        .plan
        .incoming
        .host()
        .unwrap_or_default()
        .to_owned();
    let login = run.account.plan.username();
    match asked {
        Ok(_) => Ok(()),
        Err(RuntimeError::Proto(mail_proto::ProtoError::AuthRejected(said))) => {
            let hint = if login.contains('@') {
                " Some servers want the user name without the part after the @.".to_owned()
            } else {
                String::new()
            };
            Err(CoreError::SignInRefused {
                host,
                login,
                said,
                hint,
            })
        }
        Err(other) => Err(CoreError::ServerUnreachable {
            host,
            why: other.to_string(),
        }),
    }
}
