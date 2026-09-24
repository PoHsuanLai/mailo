//! A JMAP account (RFC 8620, RFC 8621): one session URL, and everything else from it.

use super::Preset;
use crate::account::{
    AccountCaps, AccountPlan, ArchiveMeans, AuthPlan, Condstore, ConnectionBudget, ExpungeMeans,
    FolderRoles, HttpAuth, Incoming, MoveExt, Outgoing, ServerLabels, ServerThreads, Supported,
    Username, WatchMode,
};
use chrono::{DateTime, Utc};

/// Where a domain publishes its JMAP session, by RFC 8620 §2.2: `/.well-known/jmap` over HTTPS.
///
/// `None` for an address with no domain to look under.
pub fn well_known(address: &str) -> Option<String> {
    let (_, domain) = address.rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.');
    (!domain.is_empty()).then(|| format!("https://{domain}/.well-known/jmap"))
}

/// A plan for a JMAP server at `session`, signing in with a password or with a token.
///
/// Receiving and sending both go through the one server: `EmailSubmission` needs no second
/// host, no second port and no second secret, which is most of what JMAP is for.
///
/// The capabilities are a starting point dated at the epoch, so the first pass counts them as
/// stale and reads the session and the mailbox list before fetching anything — the same rule as
/// [`super::manual_pop3`]. Until then nothing is assumed: no Archive mailbox (archiving into one
/// that does not exist loses the message), no push.
///
/// Labels are [`ServerLabels::Supported`] from the start because that is a fact about the
/// protocol, not the server: every JMAP email lists the mailboxes it is in, and a mailbox that is
/// not the inbox, Sent or another role is a label in this client's sense.
pub fn jmap(address: &str, session: &str, auth: HttpAuth) -> Preset {
    Preset {
        plan: AccountPlan {
            address: address.to_owned(),
            incoming: Incoming::Jmap {
                session: session.to_owned(),
                auth,
            },
            outgoing: Outgoing::Jmap,
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                // No SASL on HTTP: `HttpAuth` says which header carries the secret.
                sasl: Vec::new(),
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Poll {
                every: super::POLL_EVERY,
            },
            archive: ArchiveMeans::LocalOnly,
            folders: FolderRoles(Vec::new()),
            condstore: Condstore::Absent,
            move_ext: MoveExt::Absent,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Absent,
            connections: ConnectionBudget { max: 2 },
            observed_at: DateTime::<Utc>::UNIX_EPOCH,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_is_looked_for_on_the_address_domain() {
        const CASES: &[(&str, Option<&str>)] = &[
            (
                "me@example.com",
                Some("https://example.com/.well-known/jmap"),
            ),
            (
                "me@Example.COM.",
                Some("https://Example.COM/.well-known/jmap"),
            ),
            ("no-domain", None),
            ("me@", None),
        ];
        for (address, want) in CASES {
            assert_eq!(well_known(address).as_deref(), *want, "{address}");
        }
    }

    #[test]
    fn a_jmap_account_sends_where_it_reads() {
        let preset = jmap(
            "me@example.com",
            "https://example.com/jmap",
            HttpAuth::Basic,
        );
        assert_eq!(preset.plan.outgoing, Outgoing::Jmap);
        assert_eq!(preset.expected_caps.labels, ServerLabels::Supported);
        // Stale on purpose: the first pass reads the session before anything else.
        assert_eq!(
            preset.expected_caps.observed_at,
            DateTime::<Utc>::UNIX_EPOCH
        );
    }
}
