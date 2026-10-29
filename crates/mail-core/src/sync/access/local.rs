//! Mail kept on this computer: no server, so nothing to fetch, search or wait on.

use super::{Access, Answer, Pass};
use crate::sync::report::Done;
use crate::sync::run::Run;
use mail_domain::{Folder, MailboxRef};

/// Local folders.
pub(crate) struct Local;

impl Access for Local {
    /// A pass over nothing: `pick` leaves these accounts out before asking for a credential, and
    /// there is nothing to do if one arrives anyway.
    fn pass<'a>(&'a self, _run: &'a Run<'a>, _pass: Pass<'a>) -> Answer<'a, Done> {
        Box::pin(std::future::ready(Ok(Done::default())))
    }

    fn mailboxes(&self, _: &crate::sync::Configured, _: &[Folder]) -> Vec<MailboxRef> {
        Vec::new()
    }
}
