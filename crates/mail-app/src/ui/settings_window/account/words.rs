//! What an account's page says: the account's servers and sign-in as rows, what Remove takes
//! with it, and why a removal did not happen. Pure, so each is a table test.

use crate::ui::copy;
use crate::ui::copy::controls::REMOVE;
use crate::ui::copy::fields::account as row;
use crate::ui::copy::status::account as said;
use crate::ui::files::work::messages;
use mail_core::account::RemoveError;
use mail_domain::{AccountPlan, AuthPlan, HttpAuth, Incoming, LeaveOnServer, Outgoing, Tls};
use porter_provider::Issuer;

/// What the Status row says while an account waits for the person.
pub(super) const WAITING: &str = said::WAITING;

/// One row of the page's settings: what it is, and how the account has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Line {
    pub label: &'static str,
    pub value: String,
}

fn line(label: &'static str, value: impl Into<String>) -> Line {
    Line {
        label,
        value: value.into(),
    }
}

fn security(tls: Tls) -> &'static str {
    match tls {
        Tls::Implicit => said::TLS,
        Tls::StartTlsRequired => said::STARTTLS,
        Tls::Plaintext => said::NOT_ENCRYPTED,
    }
}

fn server(protocol: &str, host: &str, port: u16, tls: Tls) -> String {
    said::server(protocol, host, port, security(tls))
}

fn issuer(issuer: Issuer) -> &'static str {
    match issuer {
        Issuer::Google => said::GOOGLE,
        Issuer::Microsoft => said::MICROSOFT,
        // porter names more issuers than mailo signs in to; no account here holds one.
        _ => said::ANOTHER_ISSUER,
    }
}

/// The account's settings as the page lists them, in order: receiving, what POP3 does with what
/// it downloads, sending, and signing in. The addresses it sends as are their own group, which
/// edits them (`sends_as`).
pub(super) fn settings(plan: &AccountPlan) -> Vec<Line> {
    let mut lines = Vec::new();
    let token = match &plan.incoming {
        Incoming::Imap { host, port, tls } => {
            lines.push(line(row::RECEIVING, server("IMAP", host, *port, *tls)));
            false
        }
        // What becomes of the server's copies is the page's Remove from Server, which a pass
        // reads; the plan's `leave` never decided it.
        Incoming::Pop3 {
            host, port, tls, ..
        } => {
            lines.push(line(row::RECEIVING, server("POP3", host, *port, *tls)));
            false
        }
        Incoming::Graph => {
            lines.push(line(row::RECEIVING, said::GRAPH));
            false
        }
        Incoming::Jmap { session, auth } => {
            lines.push(line(row::RECEIVING, said::jmap(&session.to_string())));
            *auth == HttpAuth::Bearer
        }
        Incoming::Local => {
            lines.push(line(row::RECEIVING, said::KEPT_HERE));
            false
        }
    };
    lines.push(line(
        row::SENDING,
        match &plan.outgoing {
            Outgoing::Smtp { host, port, tls } => server("SMTP", host, *port, *tls),
            Outgoing::Graph => said::GRAPH.to_owned(),
            Outgoing::Jmap => said::JMAP_SAME_SERVER.to_owned(),
            Outgoing::Nowhere => said::DOES_NOT_SEND.to_owned(),
        },
    ));
    lines.push(line(
        row::SIGNS_IN,
        match (&plan.auth, token) {
            (AuthPlan::OAuth { issuer: by, .. }, _) => said::in_the_browser(issuer(*by)),
            (AuthPlan::Password { .. }, true) => said::with_a_token(&plan.username().to_string()),
            (AuthPlan::Password { .. }, false) => {
                said::with_a_password(&plan.username().to_string())
            }
        },
    ));
    lines
}

/// What the confirmation says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Asking {
    /// The question, which is also the confirmation's accessible name.
    pub title: String,
    /// What goes, what stays.
    pub body: String,
    /// The button that removes.
    pub confirm: &'static str,
}

// What the confirming button says is `REMOVE`, short on purpose: the question's title already
// names the account, and the alert's two buttons share a 260px sheet, where "Remove Account" does
// not fit its button.

/// The confirmation for removing `address`, which holds `held` messages here. A POP3 server that
/// deletes what it hands over has no copy left, and the alert says so rather than "on the
/// server".
pub(super) fn asking(address: &str, held: usize, incoming: &Incoming) -> Asking {
    let mail = messages(held);
    let server = match incoming {
        Incoming::Pop3 {
            leave: LeaveOnServer::DeleteAfterFetch,
            ..
        } => copy::confirm::only_copy(&mail),
        _ => copy::confirm::MAIL_STAYS_ON_SERVER.to_owned(),
    };
    Asking {
        title: copy::confirm::remove_account(address),
        body: copy::confirm::remove_account_body(&mail, &server),
        confirm: REMOVE,
    }
}

/// Why nothing was removed, as the page says it.
pub(super) fn refused(error: &RemoveError) -> String {
    match error {
        RemoveError::Unknown => copy::errors::ACCOUNT_ALREADY_GONE.to_owned(),
        RemoveError::Local => copy::errors::LOCAL_NOT_REMOVABLE.to_owned(),
        RemoveError::Keyring(_) => copy::errors::KEYRING_WOULD_NOT_FORGET.to_owned(),
        RemoveError::Store(why) => copy::errors::nothing_removed(why),
    }
}

#[cfg(test)]
#[path = "words_tests.rs"]
mod tests;
