//! `mailo sync`, `mailo sync --folder` and `mailo watch`: how a pass is told to a terminal.
//!
//! [`mail_core::sync`] hands back how each account's pass ended, as data. The words are here, so
//! that `sync` and `watch` read a pass the same way: a watch that reported one differently from
//! the command that runs one pass would be two vocabularies for one event.

use mail_core::SqliteStore;
use mail_core::sync::report::{AccountReport, PassEnd, Watched};
use mail_domain::Retry;
use std::fmt::Write as _;

/// What `mailo sync` prints for a run over every account.
///
/// `store` is asked only when no account ran, to say which of the two reasons that was.
pub fn run_text(store: &SqliteStore, ends: &[PassEnd]) -> String {
    if ends.is_empty() {
        return nothing(!mail_core::sync::addresses(store).is_empty()).to_owned();
    }
    ends.iter()
        .map(|end| format!("{}{}", pass_text(end), sign_in_hint(store, end)))
        .collect()
}

/// What to type to sign an account in again, for a pass that was refused its sign-in; nothing for
/// any other end.
///
/// `mail-core` words the refusal without a command and classifies it ([`Retry::NeedsReauth`]);
/// how to mend it depends on how the account signs in, which the store knows.
pub fn sign_in_hint(store: &SqliteStore, end: &PassEnd) -> String {
    let address = match end {
        PassEnd::Failed {
            address,
            retry: Retry::NeedsReauth,
            ..
        } => address,
        PassEnd::Finished(report)
            if report
                .trouble
                .iter()
                .any(|t| matches!(t.retry, Retry::NeedsReauth)) =>
        {
            &report.address
        }
        _ => return String::new(),
    };
    let remedy = mail_core::sync::sign_in_remedy(store, address);
    format!(
        "  {}\n",
        super::remedy::words(&remedy).replace('\n', "\n  ")
    )
}

/// Why a run had no account to sync, given whether any account is configured at all.
///
/// An account that keeps its mail here has no server, so a store of only those has nothing to
/// fetch and says so, where a store of none says how to add one.
fn nothing(any_account: bool) -> &'static str {
    if any_account {
        "nothing to sync: the only mail here is kept on this computer\n"
    } else {
        "no accounts. Add one with: mailo account add <address>\n"
    }
}

/// How one account's pass is told to a person: what `sync` and `watch` print.
pub fn pass_text(end: &PassEnd) -> String {
    match end {
        PassEnd::Finished(report) => line(&report.address, report),
        PassEnd::Failed { address, why, .. } => format!("{address}: {why}\n"),
        PassEnd::Cancelled { address, .. } => format!("{address}: cancelled\n"),
    }
}

/// What `mailo watch` prints for each thing the watch does.
pub fn watched_text(watched: &Watched) -> String {
    match watched {
        Watched::Pass(end) => pass_text(end),
        Watched::WaitFailed { address, why } | Watched::AnnounceFailed { address, why } => {
            format!("{address}: {why}\n")
        }
        Watched::RemindersFailed { why } => format!("reminders: {why}\n"),
    }
}

/// What `mailo sync --folder` prints for a folder fetched, or the failure to exit with.
///
/// A fetch that ran is a line even when it was refused its sign-in, as it always was: the
/// refusal is among the lines it says. One that never got to run is the error.
pub fn folder_text(end: &PassEnd, path: &str) -> Result<String, String> {
    match end {
        PassEnd::Finished(report) => Ok(line(&format!("{} {path}", report.address), report)),
        PassEnd::Failed { why, .. } => Err(why.clone()),
        PassEnd::Cancelled { .. } => Err("cancelled".to_owned()),
    }
}

/// What one account's pass did, as the user reads it.
fn line(name: &str, report: &AccountReport) -> String {
    let counts = &report.counts;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{name}: {} headers, {} bodies, {} queued operations settled, {} sent",
        counts.headers_fetched, counts.bodies_fetched, counts.outbox_settled, counts.submitted
    );
    if counts.parts_fetched > 0 {
        let _ = writeln!(out, "  {} attachment(s) kept offline", counts.parts_fetched);
    }
    if !counts.ruled.is_empty() {
        let _ = writeln!(
            out,
            "  rules acted on {} new message(s)",
            counts.ruled.len()
        );
    }
    // Said plainly, because the alternative is what this used to do: someone runs `send` and
    // then `sync`, reads "0 sent", and has no reason to think their mail is still sitting here.
    // It is not an error — it will be retried — but silence reads as success.
    if counts.still_queued > 0 {
        let _ = writeln!(
            out,
            "  {} still queued; run sync again to retry, or `mailo drafts` to see why",
            counts.still_queued
        );
    }
    for note in report.trouble.iter().filter_map(|t| t.why.as_deref()) {
        let _ = writeln!(out, "  needs attention: {note}");
        // The server's words stay; this adds what they mean, where we know.
        if let Some(why) = mail_proto::diagnose_text(note).map(meaning) {
            let _ = writeln!(out, "    → {why}");
        }
    }
    out
}

/// What a diagnosed refusal means, in words a person can act on.
fn meaning(diagnosis: mail_proto::Diagnosis) -> &'static str {
    use mail_proto::Diagnosis;
    match diagnosis {
        Diagnosis::TenantSmtpAuthOff => {
            "the tenant has SMTP client authentication switched off. An administrator enables it \
             per mailbox with Set-CASMailbox -SmtpClientAuthenticationDisabled $false. Nothing \
             about the password is wrong."
        }
        Diagnosis::StartTlsFirst => "the server requires STARTTLS before authenticating.",
        Diagnosis::AppPasswordNeeded => {
            "this account has two-factor authentication, so it needs an App Password rather than \
             the account password."
        }
        Diagnosis::GoogleRejectedCredential => {
            "Google rejected the credential. Account passwords no longer work for IMAP or SMTP; \
             an App Password does."
        }
        Diagnosis::ProtocolSwitchedOff => {
            "the protocol is switched off for this mailbox. An administrator enables it; the \
             credential is not the problem."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::fetch::Pause;
    use mail_core::sync::report::{Counts, Trouble};
    use mail_domain::Retry;
    use mail_domain::id::account_id_from_uuid;
    use porter_core::AccountId;

    fn acct_account() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f1"))
    }

    fn finished(counts: Counts, trouble: Vec<Trouble>) -> PassEnd {
        PassEnd::Finished(AccountReport {
            account: acct_account(),
            address: "ada@example.test".to_owned(),
            counts,
            trouble,
        })
    }

    #[test]
    fn a_finished_account_reads_as_the_line_it_always_did() {
        let counts = Counts {
            headers_fetched: 3,
            bodies_fetched: 2,
            outbox_settled: 1,
            submitted: 1,
            parts_fetched: 4,
            still_queued: 2,
            ..Counts::default()
        };
        let trouble = vec![
            Trouble {
                mailbox: Some("INBOX".to_owned()),
                retry: Retry::Now,
                why: Some("INBOX: cannot select".to_owned()),
            },
            // Classified and silent: the line above it already said what happened.
            Trouble {
                mailbox: None,
                retry: Retry::NeedsReauth,
                why: None,
            },
        ];
        assert_eq!(
            pass_text(&finished(counts, trouble)),
            "ada@example.test: 3 headers, 2 bodies, 1 queued operations settled, 1 sent\n  \
             4 attachment(s) kept offline\n  \
             2 still queued; run sync again to retry, or `mailo drafts` to see why\n  \
             needs attention: INBOX: cannot select\n"
        );
    }

    #[test]
    fn a_run_reads_as_each_account_in_turn() {
        let failed = PassEnd::Failed {
            account: acct_account(),
            address: "bob@example.test".to_owned(),
            retry: Retry::Now,
            why: "cannot connect".to_owned(),
            pause: Pause::Unreachable,
        };
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        assert_eq!(
            run_text(&store, &[finished(Counts::default(), Vec::new()), failed]),
            "ada@example.test: 0 headers, 0 bodies, 0 queued operations settled, 0 sent\n\
             bob@example.test: cannot connect\n"
        );
    }

    #[test]
    fn syncing_with_no_accounts_explains_how_to_add_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let out = run_text(&store, &[]);
        assert!(out.contains("mailo account add"), "{out}");
    }

    #[test]
    fn a_watch_says_what_it_did_in_the_words_of_a_sync() {
        let failed = Watched::Pass(PassEnd::Failed {
            account: acct_account(),
            address: "ada@example.test".to_owned(),
            retry: Retry::Now,
            why: "cannot connect".to_owned(),
            pause: Pause::Unreachable,
        });
        assert_eq!(watched_text(&failed), "ada@example.test: cannot connect\n");
        let waited = Watched::WaitFailed {
            address: "ada@example.test".to_owned(),
            why: "connection reset".to_owned(),
        };
        assert_eq!(
            watched_text(&waited),
            "ada@example.test: connection reset\n"
        );
        let reminders = Watched::RemindersFailed {
            why: "no store".to_owned(),
        };
        assert_eq!(watched_text(&reminders), "reminders: no store\n");
    }

    #[test]
    fn a_folder_fetched_is_named_with_its_path_and_one_not_run_is_the_error() {
        let ran = finished(Counts::default(), Vec::new());
        assert_eq!(
            folder_text(&ran, "Archive/2023").unwrap(),
            "ada@example.test Archive/2023: 0 headers, 0 bodies, 0 queued operations settled, \
             0 sent\n"
        );
        let failed = PassEnd::Failed {
            account: acct_account(),
            address: "ada@example.test".to_owned(),
            retry: Retry::NeedsReauth,
            why: "not signed in".to_owned(),
            pause: Pause::ServerBusy,
        };
        assert_eq!(folder_text(&failed, "INBOX").unwrap_err(), "not signed in");
    }

    #[test]
    fn a_pass_refused_its_sign_in_is_told_how_to_sign_in_and_another_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        let ended = |retry| PassEnd::Failed {
            account: acct_account(),
            address: "ada@example.test".to_owned(),
            retry,
            why: "refused".to_owned(),
            pause: Pause::ServerBusy,
        };
        assert_eq!(
            sign_in_hint(&store, &ended(Retry::NeedsReauth)),
            "  sign in again with `mailo account add ada@example.test`\n"
        );
        assert_eq!(sign_in_hint(&store, &ended(Retry::Now)), "");
        let silent = Trouble {
            mailbox: None,
            retry: Retry::NeedsReauth,
            why: None,
        };
        assert!(
            sign_in_hint(&store, &finished(Counts::default(), vec![silent]))
                .contains("mailo account add")
        );
    }
}
