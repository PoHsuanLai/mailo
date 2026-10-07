//! What an account's page says: the account's servers and sign-in as rows, what Remove takes
//! with it, and why a removal did not happen. Pure, so each is a table test.

use crate::ui::files::work::messages;
use mail_core::account::RemoveError;
use mail_domain::{AccountPlan, AuthPlan, HttpAuth, Incoming, LeaveOnServer, Outgoing, Tls};
use porter_provider::Issuer;

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
        Tls::Implicit => "TLS",
        Tls::StartTlsRequired => "STARTTLS",
        Tls::Plaintext => "not encrypted",
    }
}

fn server(protocol: &str, host: &str, port: u16, tls: Tls) -> String {
    format!("{protocol}, {host}:{port}, {}", security(tls))
}

fn issuer(issuer: Issuer) -> &'static str {
    match issuer {
        Issuer::Google => "Google",
        Issuer::Microsoft => "Microsoft",
        // porter names more issuers than mailo signs in to; no account here holds one.
        _ => "another issuer",
    }
}

/// The account's settings as the page lists them, in order: receiving, what POP3 does with what
/// it downloads, sending, signing in, and the addresses it sends as.
pub(super) fn settings(plan: &AccountPlan) -> Vec<Line> {
    let mut lines = Vec::new();
    let token = match &plan.incoming {
        Incoming::Imap { host, port, tls } => {
            lines.push(line("Receiving", server("IMAP", host, *port, *tls)));
            false
        }
        Incoming::Pop3 {
            host,
            port,
            tls,
            leave,
        } => {
            lines.push(line("Receiving", server("POP3", host, *port, *tls)));
            lines.push(line(
                "Downloaded mail",
                match leave {
                    LeaveOnServer::Keep => "Left on the server",
                    LeaveOnServer::DeleteAfterFetch => "Deleted from the server",
                },
            ));
            false
        }
        Incoming::Graph => {
            lines.push(line("Receiving", "Microsoft Graph"));
            false
        }
        Incoming::Jmap { session, auth } => {
            lines.push(line("Receiving", format!("JMAP, {session}")));
            *auth == HttpAuth::Bearer
        }
        Incoming::Local => {
            lines.push(line("Receiving", "Kept on this computer"));
            false
        }
    };
    lines.push(line(
        "Sending",
        match &plan.outgoing {
            Outgoing::Smtp { host, port, tls } => server("SMTP", host, *port, *tls),
            Outgoing::Graph => "Microsoft Graph".to_owned(),
            Outgoing::Jmap => "JMAP, on the same server".to_owned(),
            Outgoing::Nowhere => "Does not send".to_owned(),
        },
    ));
    lines.push(line(
        "Signs in",
        match (&plan.auth, token) {
            (AuthPlan::OAuth { issuer: by, .. }, _) => {
                format!("With {}, in the browser", issuer(*by))
            }
            (AuthPlan::Password { .. }, true) => format!("With a token, as {}", plan.username()),
            (AuthPlan::Password { .. }, false) => {
                format!("With a password, as {}", plan.username())
            }
            (AuthPlan::Granted { .. }, _) => {
                "Through your system's accounts".to_owned()
            }
        },
    ));
    let addresses: Vec<String> = plan
        .identities
        .iter()
        .map(|identity| match &identity.from.name {
            Some(name) if !name.trim().is_empty() => {
                format!("{} <{}>", name.trim(), identity.from.email)
            }
            _ => identity.from.email.clone(),
        })
        .collect();
    if !addresses.is_empty() {
        lines.push(line("Sends as", addresses.join(", ")));
    }
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

/// What the confirming button says.
pub(super) const REMOVE: &str = "Remove Account";

/// The confirmation for removing `address`, which holds `held` messages here. A POP3 server that
/// deletes what it hands over has no copy left, and the alert says so rather than "on the
/// server".
pub(super) fn asking(address: &str, held: usize, incoming: &Incoming) -> Asking {
    let mail = messages(held);
    let server = match incoming {
        Incoming::Pop3 {
            leave: LeaveOnServer::DeleteAfterFetch,
            ..
        } => format!(
            "This server deletes mail once it is downloaded, so these {mail} are the only copy \
             and will be gone for good."
        ),
        _ => "Mail on the server is not touched.".to_owned(),
    };
    Asking {
        title: format!("Remove {address}?"),
        body: format!(
            "Its {mail} on this computer, its folders and rules and its saved sign-in are \
             removed from mailo. {server} Keys and certificates stay."
        ),
        confirm: REMOVE,
    }
}

/// Why nothing was removed, as the page says it.
pub(super) fn refused(error: &RemoveError) -> String {
    match error {
        RemoveError::Unknown => "This account was already removed.".to_owned(),
        RemoveError::Local => {
            "The mail kept on this computer cannot be removed like an account.".to_owned()
        }
        RemoveError::Keyring(_) => {
            "The keyring would not forget the saved sign-in, so the account and its mail were not \
             removed. It may ask you to sign in again. Unlock the keyring and try again."
                .to_owned()
        }
        RemoveError::Store(why) => format!(
            "The account and its mail were not removed: {why}. Its saved sign-in was already \
             forgotten, so it will ask you to sign in again."
        ),
    }
}

#[cfg(test)]
#[path = "words_tests.rs"]
mod tests;
