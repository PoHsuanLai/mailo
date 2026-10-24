//! The account's servers as the page edits them: a form read out of the plan, and the plan the
//! form makes again. Pure, so each is a table test.
//!
//! What can be edited is what a person types into Add Account by hand: an IMAP or POP3 server's
//! host, port and security, the SMTP server's, a JMAP account's session URL, and the user name
//! a password signs in with. What a provider decided stays read-only: Microsoft Graph has no
//! server to name, and an account that signs in in the browser signs in as its address. The
//! rest of the plan (what POP3 leaves on the server, the SASL mechanisms, the identities) is
//! carried over untouched.

use crate::ui::copy::errors;
use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing, Tls, Username};

/// One server as its rows hold it. The port is text while it is typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Server {
    pub host: String,
    pub port: String,
    pub tls: Tls,
}

impl Server {
    fn of(host: &str, port: u16, tls: Tls) -> Server {
        Server {
            host: host.to_owned(),
            port: port.to_string(),
            tls,
        }
    }

    /// The host and port, or why not, said by the row that is wrong.
    fn read(&self) -> Result<(String, u16), Refusal> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err(refused(Row::Host, errors::NO_HOST));
        }
        match self.port.trim().parse::<u16>() {
            Ok(port) if port > 0 => Ok((host.to_owned(), port)),
            _ => Err(refused(Row::Port, errors::BAD_PORT)),
        }
    }
}

/// What the incoming side of the form edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Receiving {
    Server(Server),
    Session(String),
    /// Graph, or mail kept here: nothing to edit.
    Fixed,
}

/// Every editable row of the servers group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ServerForm {
    pub incoming: Receiving,
    /// `None` where the account sends through Graph, JMAP or not at all.
    pub outgoing: Option<Server>,
    /// `None` where the account signs in in the browser.
    pub username: Option<String>,
}

/// A row of the servers group, for saying which one is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    Host,
    Port,
    OutgoingHost,
    OutgoingPort,
    Username,
}

/// Which row the form is refused at, and the words drawn under it: the form's own, or the
/// server's when the sign-in with the new settings did not go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Refusal {
    pub row: Row,
    pub why: String,
}

fn refused(row: Row, why: &str) -> Refusal {
    Refusal {
        row,
        why: why.to_owned(),
    }
}

/// Where the check's refusal is drawn: a login the server turned down under the user name, and
/// anything else (a server that does not answer, a store that would not write) under the host,
/// the likeliest thing typed wrong.
pub(super) fn refusal_of(error: &mail_core::CoreError) -> Refusal {
    let row = match error {
        mail_core::CoreError::SignInRefused { .. } => Row::Username,
        _ => Row::Host,
    };
    refused(row, &error.to_string())
}

/// Whether the account's page has any server row to edit.
pub(super) fn editable(form: &ServerForm) -> bool {
    form.incoming != Receiving::Fixed || form.outgoing.is_some() || form.username.is_some()
}

/// The form for `plan`, as the account has it now.
pub(super) fn form_of(plan: &AccountPlan) -> ServerForm {
    let incoming = match &plan.incoming {
        Incoming::Imap { host, port, tls }
        | Incoming::Pop3 {
            host, port, tls, ..
        } => Receiving::Server(Server::of(host, *port, *tls)),
        Incoming::Jmap { session, .. } => Receiving::Session(session.clone()),
        Incoming::Graph | Incoming::Local => Receiving::Fixed,
    };
    let outgoing = match &plan.outgoing {
        Outgoing::Smtp { host, port, tls } => Some(Server::of(host, *port, *tls)),
        Outgoing::Graph | Outgoing::Jmap | Outgoing::Nowhere => None,
    };
    let username = match &plan.auth {
        AuthPlan::Password { .. } if !matches!(plan.incoming, Incoming::Local) => {
            Some(plan.username())
        }
        AuthPlan::Password { .. } | AuthPlan::OAuth { .. } => None,
    };
    ServerForm {
        incoming,
        outgoing,
        username,
    }
}

/// `plan` with what `form` says, or the first row that is wrong.
///
/// A user name typed back as the one the account already resolves to keeps the way it was
/// written (the address, its local part), so saving an untouched form changes nothing.
pub(super) fn plan_from(plan: &AccountPlan, form: &ServerForm) -> Result<AccountPlan, Refusal> {
    let mut out = plan.clone();
    match (&mut out.incoming, &form.incoming) {
        (
            Incoming::Imap { host, port, tls }
            | Incoming::Pop3 {
                host, port, tls, ..
            },
            Receiving::Server(server),
        ) => {
            (*host, *port) = server.read()?;
            *tls = server.tls;
        }
        (Incoming::Jmap { session, .. }, Receiving::Session(typed)) => {
            let typed = typed.trim();
            if typed.is_empty() {
                return Err(refused(Row::Host, errors::NO_HOST));
            }
            *session = typed.to_owned();
        }
        _ => {}
    }
    if let (Outgoing::Smtp { host, port, tls }, Some(server)) = (&mut out.outgoing, &form.outgoing)
    {
        (*host, *port) = server.read().map_err(|wrong| Refusal {
            row: match wrong.row {
                Row::Host => Row::OutgoingHost,
                _ => Row::OutgoingPort,
            },
            why: wrong.why,
        })?;
        *tls = server.tls;
    }
    if let (AuthPlan::Password { username, .. }, Some(typed)) = (&mut out.auth, &form.username) {
        let typed = typed.trim();
        if typed.is_empty() {
            return Err(refused(Row::Username, errors::NO_USER_NAME));
        }
        if typed != username.resolve(&plan.address) {
            *username = if typed == plan.address {
                Username::SameAsAddress
            } else {
                Username::Literal(typed.to_owned())
            };
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "servers_tests.rs"]
mod tests;
