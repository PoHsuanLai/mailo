//! How a message leaves an account, as far as the composer asks: how large a message the way
//! takes, and whether it can ask for delivery reports.
//!
//! [`Outgoing`] is the stored name of a way of sending and stays an enum. A [`Submit`] is the
//! way it names, and [`of`] is the one place in this crate that matches on it. What the engine
//! does to send is `mail_runtime`'s, and names every case of [`Outgoing`] where it does.

use super::weight::{Measure, Said, SendLimit};
use mail_domain::Outgoing;

/// One way of sending.
pub(super) trait Submit: Sync {
    /// The largest message this way takes, given what the sending server last said (`stored`).
    fn limit(&self, stored: Option<u64>) -> SendLimit {
        match stored {
            Some(octets) => SendLimit {
                octets,
                measure: Measure::Wire,
                said: Said::Server,
            },
            None => SendLimit::USUAL,
        }
    }

    /// Whether a message can ask for delivery reports (RFC 3461).
    fn asks_delivery_reports(&self) -> bool {
        true
    }
}

/// SMTP: the server's `SIZE`, once it has said it.
struct Smtp;

impl Submit for Smtp {}

/// JMAP's `EmailSubmission`: the limit the session said, kept as an SMTP server's is.
struct Jmap;

impl Submit for Jmap {}

/// Microsoft Graph's `sendMail`: what its upload sessions take, and no delivery reports.
struct Graph;

impl Submit for Graph {
    fn limit(&self, _stored: Option<u64>) -> SendLimit {
        SendLimit {
            octets: mail_runtime::graph::MAX_MESSAGE,
            measure: Measure::Content,
            said: Said::Graph,
        }
    }

    fn asks_delivery_reports(&self) -> bool {
        false
    }
}

/// An account that sends nowhere: the usual limit, whatever was stored.
struct Nowhere;

impl Submit for Nowhere {
    fn limit(&self, _stored: Option<u64>) -> SendLimit {
        SendLimit::USUAL
    }
}

/// The way `outgoing` names. The only match on [`Outgoing`] in this crate.
pub(super) fn of(outgoing: &Outgoing) -> &'static dyn Submit {
    match outgoing {
        Outgoing::Smtp { .. } => &Smtp,
        Outgoing::Graph => &Graph,
        Outgoing::Jmap => &Jmap,
        Outgoing::Nowhere => &Nowhere,
    }
}

/// The way of an account whose plan does not read: through the server that said what it takes,
/// with the choice of delivery reports offered.
pub(super) fn unread() -> &'static dyn Submit {
    &Smtp
}
