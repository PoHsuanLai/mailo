//! What to type about a failure: [`mail_core::Remedy`] in the command line's words.
//!
//! `mail-core` says which step mends a failure and names no command; this is where the terminal
//! turns it into one. A failure is told as its own words, a semicolon, then the step.

use mail_core::error::CoreError;
use mail_core::{Remedy, SignInWith};
use porter_provider::Issuer;

/// The step, as a clause that reads after the failure's own words.
pub fn words(remedy: &Remedy) -> String {
    match remedy {
        Remedy::Sync => "run `mailo sync` and try again".to_owned(),
        Remedy::AddAccount => "`mailo account add <address>` adds one".to_owned(),
        Remedy::ReAddAccount => "`mailo account add <address>` adds it again".to_owned(),
        Remedy::ListAccounts => "`mailo account list` says which there are".to_owned(),
        Remedy::Folders { address, folder } => format!(
            "`mailo folder list {address}` shows them, and `mailo folder new {address} \
             {folder}` makes one"
        ),
        Remedy::StopDaemon => "`mailo daemon --stop` will stop it".to_owned(),
        Remedy::SaveInvitation => "save it with: mailo invite <message-id> --ics FILE".to_owned(),
        Remedy::NameJmapSession { address } => format!(
            "name it:\n\n  mailo account add {address} --jmap \
             https://jmap.example.com/.well-known/jmap"
        ),
        Remedy::NameServers { address } => format!(
            "name them:\n\n  mailo account add {address} --imap imap.example.com --smtp \
             smtp.example.com\n\n\
             Ports default to 993 and 465, both with implicit TLS. A server that offers \
             only POP3 takes --pop3 in place of --imap (port 995). Add --login NAME if \
             the server wants something other than the whole address."
        ),
        Remedy::SignIn { address, with } => sign_in(address, *with),
        Remedy::FindPgpKey => {
            "`mailo pgp lookup <address>` finds one and `mailo pgp import <file>` imports one"
                .to_owned()
        }
        Remedy::MakePgpKey { address } => format!("make one with `mailo pgp generate {address}`"),
        Remedy::ExportPgpSecret { fingerprint } => format!(
            "export it with `mailo pgp export {fingerprint} --secret`, then delete with \
             --with-secret"
        ),
        Remedy::ImportCertificate => "`mailo smime import <file>` imports one".to_owned(),
        Remedy::ImportOwnCertificate => "`mailo smime import <file.p12>` imports it".to_owned(),
    }
}

/// How to sign in, which depends on how the account does: a password account supplies the
/// password, an OAuth one a client id (and Google's secret), and Microsoft's needs `--microsoft`
/// for the command to reproduce the account at all. Advice that fails when followed is worse than
/// none, so the command carries exactly what that account needs.
fn sign_in(address: &str, with: SignInWith) -> String {
    match with {
        SignInWith::Password => {
            format!("run:\n    MAILO_PASSWORD=… mailo account add {address}")
        }
        SignInWith::OAuth { issuer } => {
            let flag = match issuer {
                Issuer::Microsoft => " --microsoft",
                _ => "",
            };
            // Google will not exchange a code without the application secret it issued.
            let secret = match issuer {
                Issuer::Google => "MAILO_OAUTH_CLIENT_SECRET=… ",
                _ => "",
            };
            format!("run:\n    MAILO_OAUTH_CLIENT_ID=… {secret}mailo account add {address}{flag}")
        }
        SignInWith::Unknown => format!("sign in again with `mailo account add {address}`"),
    }
}

/// A core failure as the terminal says it: its own words, then the step that mends it, if any.
///
/// Anything that converts into a [`CoreError`] will do, so a call site that holds a store or a
/// runtime error says it the same way.
pub fn error(failure: impl Into<CoreError>) -> String {
    told(&failure.into())
}

/// [`error`] for a failure held by reference.
pub fn told(failure: &CoreError) -> String {
    match failure.remedy() {
        Some(remedy) => format!("{failure}; {}", words(&remedy)),
        None => failure.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::AuthPlan;

    fn told(auth: AuthPlan) -> String {
        error(CoreError::NoCredential {
            address: "ada@example.test".to_owned(),
            auth,
        })
    }

    #[test]
    fn a_password_account_is_told_about_the_password() {
        let out = told(AuthPlan::Password {
            username: mail_domain::Username::SameAsAddress,
            sasl: vec![mail_domain::SaslMech::Plain],
        });
        assert!(out.contains("no credential stored"), "{out}");
        assert!(out.contains("MAILO_PASSWORD"), "{out}");
        // The address, not the word "<address>": advice that has to be edited before it can be
        // run is advice someone gets wrong at the point they are least able to tell.
        assert!(out.contains("mailo account add ada@example.test"), "{out}");
        assert!(!out.contains("<address>"), "{out}");
    }

    #[test]
    fn an_oauth_account_is_not_sent_to_find_a_password() {
        let out = told(AuthPlan::OAuth {
            issuer: Issuer::Google,
            scopes: vec!["https://mail.google.com/".to_owned()],
        });
        assert!(
            !out.contains("MAILO_PASSWORD"),
            "a Google account was told to set a password, which Google has not accepted since \
             2022: {out}"
        );
        assert!(out.contains("MAILO_OAUTH_CLIENT_ID"), "{out}");
        assert!(out.contains("MAILO_OAUTH_CLIENT_SECRET"), "{out}");
        assert!(out.contains("mailo account add ada@example.test"), "{out}");
        // Read by a person, and `cargo fmt` collapses a `\`-continuation in a literal into a run
        // of spaces in the middle of the sentence. The indented command is the one place a run
        // of spaces is meant.
        let sentence = out.split_once(";").map_or(out.as_str(), |(head, _)| head);
        assert!(
            !sentence.contains("  "),
            "a run of spaces in a message: {out:?}"
        );
    }

    /// Microsoft needs `--microsoft` to reproduce the account, and an instruction that does not
    /// work when followed is worse than none.
    #[test]
    fn a_microsoft_account_keeps_the_flag_that_makes_the_command_work() {
        let out = told(AuthPlan::OAuth {
            issuer: Issuer::Microsoft,
            scopes: vec!["https://outlook.office.com/IMAP.AccessAsUser.All".to_owned()],
        });
        assert!(
            out.contains("mailo account add ada@example.test --microsoft"),
            "{out}"
        );
        assert!(!out.contains("MAILO_OAUTH_CLIENT_SECRET"), "{out}");
    }

    #[test]
    fn a_failure_with_no_step_is_only_its_own_words() {
        assert_eq!(
            error(CoreError::NoMessageId),
            CoreError::NoMessageId.to_string()
        );
    }

    /// The class: nothing mail-core says contains a command, and each step that had one in its
    /// words is a remedy now, so the terminal still names it.
    #[test]
    fn a_step_core_used_to_spell_out_is_worded_here() {
        for (failure, needle) in [
            (CoreError::NoAccounts, "mailo account add"),
            (CoreError::HeadersOnly, "mailo sync"),
            (CoreError::NotFetchedInFull, "mailo sync"),
            (CoreError::ThreadNotFetchedInFull, "mailo sync"),
            (CoreError::NotDownloaded, "mailo sync"),
            (CoreError::PublishedEvent, "mailo invite"),
            (CoreError::DaemonRunning, "mailo daemon --stop"),
            (CoreError::NoIdentityToSendAs, "mailo account add"),
            (
                CoreError::NoAccountFor {
                    address: "a@example.test".to_owned(),
                },
                "mailo account list",
            ),
            (
                CoreError::NoFolder {
                    address: "a@example.test".to_owned(),
                    folder: "Old".to_owned(),
                },
                "mailo folder new a@example.test Old",
            ),
            (
                CoreError::NoPreset {
                    address: "a@example.test".to_owned(),
                },
                "--imap imap.example.com",
            ),
            (
                CoreError::NoJmapSession {
                    address: "a@example.test".to_owned(),
                },
                "--jmap https://jmap.example.com",
            ),
        ] {
            let plain = failure.to_string();
            assert!(!plain.contains("mailo "), "core names a command: {plain}");
            let said = error(failure);
            assert!(said.starts_with(&plain), "{said}");
            assert!(said.contains(needle), "{said}");
        }
    }
}
