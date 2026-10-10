//! What to do about a failure: [`mail_core::Remedy`] in the window's words.
//!
//! `mail-core` says which step mends a failure and names no command; the terminal words it as
//! one to type (`cli::remedy`), and this words it as a place to go and a thing to do, because a
//! person in the window has no command line to type into. A failure is told as its own words,
//! then the step as a sentence of its own.

use mail_core::error::CoreError;
use mail_core::{Remedy, SignInWith};

/// The step, as a sentence without its full stop, so it reads after the failure's own words.
pub(in crate::ui) fn words(remedy: &Remedy) -> String {
    match remedy {
        Remedy::Sync => "Sync, then try again".to_owned(),
        Remedy::AddAccount => "Add an account in Settings".to_owned(),
        Remedy::ReAddAccount => "Add the account again in Settings".to_owned(),
        Remedy::ListAccounts => "Check the address against your accounts in Settings".to_owned(),
        Remedy::Folders { folder, .. } => {
            format!("Choose a folder that exists, or make \u{201c}{folder}\u{201d} first")
        }
        Remedy::StopDaemon => "Stop the daemon that is running".to_owned(),
        Remedy::SaveInvitation => "Save the invitation to a file instead".to_owned(),
        Remedy::NameJmapSession { .. } => "Enter the JMAP session address".to_owned(),
        Remedy::NameServers { .. } => "Enter the names of the mail servers".to_owned(),
        Remedy::SignIn { address, with } => match with {
            SignInWith::Password => format!("Enter the password for {address} in Settings"),
            SignInWith::OAuth { .. } | SignInWith::Unknown => {
                format!("Sign in to {address} again in Settings")
            }
        },
        Remedy::FindPgpKey => "Find or import a key for each recipient".to_owned(),
        Remedy::MakePgpKey { address } => format!("Make a key for {address} first"),
        Remedy::ExportPgpSecret { .. } => "Export the secret key first".to_owned(),
        Remedy::ImportCertificate => "Import a certificate for each recipient".to_owned(),
        Remedy::ImportOwnCertificate => "Import a certificate of your own".to_owned(),
    }
}

/// A core failure as the window says it: its own words, then the step that mends it, if any.
///
/// Anything that converts into a [`CoreError`] will do, as for the terminal's `remedy::error`.
pub(in crate::ui) fn told(failure: impl Into<CoreError>) -> String {
    let failure = failure.into();
    match failure.remedy() {
        Some(remedy) => format!(
            "{}. {}.",
            failure.to_string().trim_end_matches('.'),
            words(&remedy)
        ),
        None => failure.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{AuthPlan, SaslMech, Username};
    use porter_provider::Issuer;

    fn address() -> String {
        "ada@example.test".to_owned()
    }

    /// One of each. `words` matches without a wildcard, so a new variant is a compile error there
    /// and then a line to add here.
    fn every() -> Vec<Remedy> {
        vec![
            Remedy::Sync,
            Remedy::AddAccount,
            Remedy::ReAddAccount,
            Remedy::ListAccounts,
            Remedy::Folders {
                address: address(),
                folder: "Old".to_owned(),
            },
            Remedy::StopDaemon,
            Remedy::SaveInvitation,
            Remedy::NameJmapSession { address: address() },
            Remedy::NameServers { address: address() },
            Remedy::SignIn {
                address: address(),
                with: SignInWith::Password,
            },
            Remedy::SignIn {
                address: address(),
                with: SignInWith::OAuth {
                    issuer: Issuer::Google,
                },
            },
            Remedy::SignIn {
                address: address(),
                with: SignInWith::Unknown,
            },
            Remedy::FindPgpKey,
            Remedy::MakePgpKey { address: address() },
            Remedy::ExportPgpSecret {
                fingerprint: "0123456789abcdef0123456789abcdef01234567"
                    .parse()
                    .expect("a fingerprint"),
            },
            Remedy::ImportCertificate,
            Remedy::ImportOwnCertificate,
        ]
    }

    /// The class: every step has window wording, and none of it is the terminal's. A person in the
    /// window cannot type `mailo sync`, so a step that read like one would be advice they cannot
    /// follow.
    #[test]
    fn every_remedy_has_wording_for_the_window_and_none_names_a_command() {
        for remedy in every() {
            let said = words(&remedy);
            assert!(!said.is_empty(), "{remedy:?}");
            assert!(
                !said.contains("mailo") && !said.contains('`') && !said.contains("--"),
                "a command in the window's words for {remedy:?}: {said}"
            );
            assert!(
                said.chars().next().is_some_and(char::is_uppercase)
                    && !said.ends_with('.')
                    && !said.contains("  "),
                "{remedy:?}: {said:?}"
            );
        }
    }

    #[test]
    fn a_failure_with_a_step_is_its_words_then_the_step() {
        let said = told(CoreError::HeadersOnly);
        assert_eq!(
            said,
            format!(
                "{}. Sync, then try again.",
                CoreError::HeadersOnly.to_string().trim_end_matches('.')
            )
        );
    }

    #[test]
    fn a_failure_with_no_step_is_only_its_own_words() {
        assert_eq!(
            told(CoreError::NoMessageId),
            CoreError::NoMessageId.to_string()
        );
    }

    #[test]
    fn the_step_follows_how_the_account_signs_in() {
        let password = told(CoreError::NoCredential {
            address: address(),
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
        });
        assert!(password.ends_with("Enter the password for ada@example.test in Settings."));
        let oauth = told(CoreError::NoCredential {
            address: address(),
            auth: AuthPlan::OAuth {
                issuer: Issuer::Google,
                scopes: vec![],
            },
        });
        assert!(oauth.ends_with("Sign in to ada@example.test again in Settings."));
    }

    /// The same failures the terminal words (`cli::remedy`) are worded here, and core's own text
    /// names no command in either.
    #[test]
    fn what_the_terminal_words_the_window_words_too() {
        for failure in [
            CoreError::NoAccounts,
            CoreError::HeadersOnly,
            CoreError::NotDownloaded,
            CoreError::PublishedEvent,
            CoreError::DaemonRunning,
            CoreError::NoIdentityToSendAs,
            CoreError::NoAccountFor { address: address() },
            CoreError::NoPreset { address: address() },
        ] {
            let plain = failure.to_string();
            let said = told(failure);
            assert!(said.len() > plain.len(), "no step after: {said}");
            assert!(!said.contains("mailo "), "{said}");
        }
    }
}
