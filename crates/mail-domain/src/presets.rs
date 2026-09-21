//! Domain-to-configuration presets. Data, not types.
//!
//! Nothing here is a domain concept. A preset fills an [`AccountPlan`] and supplies the
//! capabilities we *expect*; the runtime replaces those with what the server actually
//! advertises. This is the only place in the workspace that may mention a specific provider,
//! and even then only as a table key.

use crate::account::{
    AccountCaps, AccountPlan, ArchiveMeans, AuthPlan, Condstore, FolderRoles, Incoming,
    LeaveOnServer, MoveExt, OAuthIssuer, Outgoing, SaslMech, ServerLabels, ServerThreads, Tls,
    Username, WatchMode,
};
use crate::state::MailboxRole;
use chrono::{DateTime, Utc};
use std::time::Duration;

/// A preset's contribution: configuration, plus a starting guess at capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub plan: AccountPlan,
    /// Replaced on first connect. Only a starting value, so the first sync has something to
    /// work with before `CAPABILITY` comes back.
    pub expected_caps: AccountCaps,
}

// ---------------------------------------------------------------------------------------
// The table.
//
// Everything in `expected_caps` below is an EXPECTED value, not a fact. The runtime replaces
// the whole `AccountCaps` with what `CAPABILITY` / `CAPA` / `LIST (SPECIAL-USE)` report on
// the first connect; these exist only so the first sync has something to plan against before
// that round trip completes. A preset that turns out to be wrong is a stale guess, never a
// persisted lie — `AccountPlan` (configured) and `AccountCaps` (discovered) are separate
// types for exactly this reason.
//
// This module is also the ONLY place in the workspace that may name a provider. Domain types
// stay vendor-neutral (`ArchiveMeans::DropInbox`, not `GmailStyle`); the single exception is
// `OAuthIssuer::Google`, which names an authorization server rather than a mail provider.
//
// `identities` is always empty here. An `Identity` needs an `IdentityId` and an `AccountId`,
// and a preset has neither: minting one would make `preset_for` impure (a fresh UUID per
// call, so two calls with the same arguments differ) and would invent an account id that no
// store row matches. The account-creation flow mints the `AccountId`, builds the default
// `Identity` from the address the user typed, and pushes it onto this empty vector.
// ---------------------------------------------------------------------------------------

/// Gmail's IMAP/SMTP OAuth scope. `https://mail.google.com/` is the only scope that grants
/// IMAP and SMTP access; `email` is requested alongside it so the callback can confirm which
/// account actually consented, which is the address we key everything else off.
const GMAIL_SCOPES: [&str; 2] = ["https://mail.google.com/", "email"];

/// Gmail's special-use paths. Sent as `FolderRoles` so the first sync can address
/// `[Gmail]/All Mail` before `LIST (SPECIAL-USE)` comes back. Gmail localises these paths
/// for some accounts, which is one more reason the runtime overwrites them.
fn gmail_folders() -> FolderRoles {
    FolderRoles(vec![
        ("INBOX".to_owned(), MailboxRole::Inbox),
        ("[Gmail]/All Mail".to_owned(), MailboxRole::Archive),
        ("[Gmail]/Sent Mail".to_owned(), MailboxRole::Sent),
        ("[Gmail]/Drafts".to_owned(), MailboxRole::Drafts),
        ("[Gmail]/Trash".to_owned(), MailboxRole::Trash),
        ("[Gmail]/Spam".to_owned(), MailboxRole::Spam),
    ])
}

/// How often to poll a server with no `IDLE`. Five minutes: often enough to feel live,
/// rare enough not to look like abuse to a campus server.
const NTU_POLL_EVERY: Duration = Duration::from_secs(5 * 60);

/// NTU's two POP3 front ends. Separated into constants because a live spike is running
/// against them now and either host, or the rule that picks between them, may need to change
/// without touching anything else in this file.
const NTU_MSA_HOST: &str = "msa.ntu.edu.tw";
const NTU_CCMS_HOST: &str = "ccms.ntu.edu.tw";
const NTU_SMTP_HOST: &str = "smtps.ntu.edu.tw";

/// Look up configuration for an address.
///
/// Returns `None` for an unknown domain, which the UI turns into a manual setup form. Some
/// domains need the local part as well: `ntu.edu.tw` picks between `msa` and `ccms` from the
/// shape of the local part, which is preset logic and belongs here rather than in a type.
///
/// The domain is matched case-insensitively (ASCII: mail domains are IDNA-encoded by the
/// time they reach us). An address with no `@`, or with an empty local part or domain, has no
/// preset and returns `None` rather than panicking — this string comes from a text field.
pub fn preset_for(address: &str, now: DateTime<Utc>) -> Option<Preset> {
    // Split at the LAST `@`: a quoted local part may legally contain one.
    let (local, domain) = address.rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    let domain = domain.to_ascii_lowercase();

    match domain.as_str() {
        "gmail.com" | "googlemail.com" => Some(gmail(address, now)),
        "ntu.edu.tw" => Some(ntu(address, local, now)),
        _ => None,
    }
}

fn gmail(address: &str, now: DateTime<Utc>) -> Preset {
    Preset {
        plan: AccountPlan {
            address: address.to_owned(),
            incoming: Incoming::Imap {
                host: "imap.gmail.com".to_owned(),
                port: 993,
                tls: Tls::Implicit,
            },
            outgoing: Outgoing::Smtp {
                host: "smtp.gmail.com".to_owned(),
                port: 465,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                scopes: GMAIL_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::ProviderId,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::DropInbox,
            folders: gmail_folders(),
            condstore: Condstore::Supported,
            move_ext: MoveExt::Supported,
            observed_at: now,
        },
    }
}

fn ntu(address: &str, local: &str, now: DateTime<Utc>) -> Preset {
    Preset {
        plan: AccountPlan {
            address: address.to_owned(),
            incoming: Incoming::Pop3 {
                host: ntu_pop_host(local).to_owned(),
                port: 995,
                tls: Tls::Implicit,
                // Keep, always. POP3 with `DeleteAfterFetch` makes this client the only copy
                // of the user's mail, and a first sync that is interrupted halfway then loses
                // whatever it had already deleted.
                leave: LeaveOnServer::Keep,
            },
            outgoing: Outgoing::Smtp {
                host: NTU_SMTP_HOST.to_owned(),
                port: 465,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::Password {
                // `user@ntu.edu.tw` logs in as `user`.
                username: Username::LocalPart,
                // `Login` first: the spike shows the campus server offering `LOGIN`, and
                // `Plain` is the fallback for the hosts that do not.
                // Measured against msa.ntu.edu.tw on 2026-09-22: CAPA advertises
                // SASL PLAIN and USER, and does NOT offer LOGIN or CRAM-MD5.
                // Offering LOGIN first would have failed on the first connect.
                sasl: vec![SaslMech::Plain],
            },
            identities: Vec::new(),
        },
        expected_caps: AccountCaps {
            // POP3 has no keywords and no folders at all, so every one of these is local.
            labels: ServerLabels::LocalOnly,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Poll {
                every: NTU_POLL_EVERY,
            },
            archive: ArchiveMeans::LocalOnly,
            // POP3 exposes one implicit maildrop, so there is no path-to-role table to fill.
            folders: FolderRoles(Vec::new()),
            condstore: Condstore::Absent,
            move_ext: MoveExt::Absent,
            observed_at: now,
        },
    }
}

/// Which NTU POP3 front end serves this local part.
///
/// **The heuristic:** a local part shaped like an NTU student/staff id — one or two ASCII
/// letters followed by seven to nine ASCII digits (`b09901123`, `r10922001`, `d08944002`), or
/// all digits — is served by [`NTU_MSA_HOST`]. Anything else is treated as a name-shaped
/// account (`chenyuting`, `y.t.chen`, `prof-lin`) and served by [`NTU_CCMS_HOST`].
///
/// This is a guess about an account-naming convention, not a protocol fact, and a live spike
/// is checking it right now. It is one predicate and two constants on purpose: if the spike
/// says the split runs the other way, or that one host serves everyone, the correction is
/// this function alone. When the guess is wrong the symptom is a POP3 login failure on the
/// first connect, not silent data loss — and the setup UI can still offer the other host.
///
/// Matching is ASCII-only and case-insensitive; NTU ids are ASCII by construction.
fn ntu_pop_host(local: &str) -> &'static str {
    if looks_like_ntu_id(local) {
        NTU_MSA_HOST
    } else {
        NTU_CCMS_HOST
    }
}

/// Whether a local part has the shape of an NTU id. See [`ntu_pop_host`].
///
/// The `local[letters..]` slice below is a byte index built from a *character* count. That is
/// sound only because `is_ascii_alphabetic` accepts nothing wider than one byte. Widening the
/// predicate to `is_alphabetic` would make it panic on a non-ASCII local part — which is
/// attacker-supplied — so keep it ASCII, or switch to `char_indices`.
fn looks_like_ntu_id(local: &str) -> bool {
    let letters = local
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .count();
    if letters > 2 {
        return false;
    }
    let digits = &local[letters..];
    (7..=9).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
            .expect("literal is valid RFC 3339")
            .with_timezone(&Utc)
    }

    #[test]
    fn gmail_domains_and_case() {
        for address in [
            "someone@gmail.com",
            "someone@GMAIL.com",
            "someone@googlemail.com",
            "Someone@GoogleMail.COM",
        ] {
            let preset = preset_for(address, at()).unwrap_or_else(|| panic!("{address}"));
            assert_eq!(
                preset.plan.incoming,
                Incoming::Imap {
                    host: "imap.gmail.com".to_owned(),
                    port: 993,
                    tls: Tls::Implicit,
                },
                "{address}"
            );
            // The address is kept exactly as typed; only the lookup folds case.
            assert_eq!(preset.plan.address, address, "{address}");
            assert_eq!(preset.expected_caps.observed_at, at(), "{address}");
            assert_eq!(
                preset.expected_caps.folders.path(MailboxRole::Archive),
                Some("[Gmail]/All Mail"),
                "{address}"
            );
        }
    }

    #[test]
    fn ntu_host_selection() {
        const CASES: &[(&str, &str)] = &[
            ("b09901123@ntu.edu.tw", NTU_MSA_HOST),
            ("r10922001@ntu.edu.tw", NTU_MSA_HOST),
            ("d08944002@ntu.edu.tw", NTU_MSA_HOST),
            ("B09901123@NTU.EDU.TW", NTU_MSA_HOST),
            ("123456789@ntu.edu.tw", NTU_MSA_HOST),
            ("chenyuting@ntu.edu.tw", NTU_CCMS_HOST),
            ("y.t.chen@ntu.edu.tw", NTU_CCMS_HOST),
            ("prof-lin@ntu.edu.tw", NTU_CCMS_HOST),
            ("b099@ntu.edu.tw", NTU_CCMS_HOST),
            ("abc09901123@ntu.edu.tw", NTU_CCMS_HOST),
        ];
        for (address, expected) in CASES {
            let preset = preset_for(address, at()).unwrap_or_else(|| panic!("{address}"));
            let Incoming::Pop3 {
                host,
                port,
                tls,
                leave,
            } = preset.plan.incoming
            else {
                panic!("{address}: expected POP3");
            };
            assert_eq!(host, *expected, "{address}");
            assert_eq!(port, 995, "{address}");
            assert_eq!(tls, Tls::Implicit, "{address}");
            assert_eq!(leave, LeaveOnServer::Keep, "{address}");
        }
    }

    #[test]
    fn ntu_auth_and_caps() {
        let preset = preset_for("b09901123@ntu.edu.tw", at()).expect("ntu preset");
        assert_eq!(
            preset.plan.auth,
            AuthPlan::Password {
                username: Username::LocalPart,
                sasl: vec![SaslMech::Plain],
            }
        );
        assert_eq!(
            preset.plan.outgoing,
            Outgoing::Smtp {
                host: NTU_SMTP_HOST.to_owned(),
                port: 465,
                tls: Tls::Implicit,
            }
        );
        assert_eq!(preset.expected_caps.archive, ArchiveMeans::LocalOnly);
        assert_eq!(preset.expected_caps.labels, ServerLabels::LocalOnly);
        assert_eq!(preset.expected_caps.threads, ServerThreads::Jwz);
        assert_eq!(
            preset.expected_caps.watch,
            WatchMode::Poll {
                every: NTU_POLL_EVERY
            }
        );
        assert_eq!(preset.expected_caps.condstore, Condstore::Absent);
        assert_eq!(preset.expected_caps.move_ext, MoveExt::Absent);
        assert!(preset.expected_caps.folders.0.is_empty());
        assert!(preset.plan.identities.is_empty());
    }

    #[test]
    fn unknown_and_malformed() {
        const CASES: &[&str] = &[
            "someone@example.com",
            "someone@sub.gmail.com",
            "someone@gmail.com.evil.test",
            "not-an-address",
            "",
            "@gmail.com",
            "someone@",
            "@",
        ];
        for address in CASES {
            assert!(
                preset_for(address, at()).is_none(),
                "{address} should have no preset"
            );
        }
    }

    #[test]
    fn last_at_wins() {
        // A quoted local part may contain `@`; the domain is what follows the last one.
        let preset = preset_for("\"odd@name\"@gmail.com", at());
        assert!(preset.is_some());
    }
}
