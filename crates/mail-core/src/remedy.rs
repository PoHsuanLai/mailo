//! What a person can do about a failure, as a value.
//!
//! A failure's `Display` says what went wrong in words that fit any front end. What to *do*
//! about it differs: the command line names a command to type, the window names a button.
//! So core says which remedy applies and the front end words it (`mail-app`'s `said::remedy`).
//! `mail-core` never names a command.

use mail_domain::Fingerprint;
use porter_provider::Issuer;

/// How an account signs in, for the one remedy whose steps depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInWith {
    /// A password in the keyring.
    Password,
    /// An OAuth token, which needs a client id registered with the issuer.
    OAuth { issuer: Issuer },
    /// Not known where this is said: sign in again and be told what else is needed.
    Unknown,
}

impl SignInWith {
    /// How an account's plan signs in. A grant is the desktop's to give again, so it names no
    /// way of ours.
    pub fn of(auth: &mail_domain::AuthPlan) -> Self {
        match auth {
            mail_domain::AuthPlan::OAuth { issuer, .. } => SignInWith::OAuth { issuer: *issuer },
            mail_domain::AuthPlan::Password { .. } => SignInWith::Password,
            mail_domain::AuthPlan::Granted { .. } => SignInWith::Unknown,
        }
    }
}

/// The step that mends a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    /// Fetch the mail first, then try again.
    Sync,
    /// There is no account yet: add one.
    AddAccount,
    /// The account was added before something existed: add it again.
    ReAddAccount,
    /// The account named is not one there is: see which there are.
    ListAccounts,
    /// The folder named is not one there is: see which there are, or make it.
    Folders { address: String, folder: String },
    /// A daemon already runs: stop it.
    StopDaemon,
    /// An invitation that asks for no answer can be saved to a file.
    SaveInvitation,
    /// The JMAP session URL has to be named.
    NameJmapSession { address: String },
    /// No preset covers the address: the servers have to be named.
    NameServers { address: String },
    /// The account has to sign in, again or for the first time.
    SignIn { address: String, with: SignInWith },
    /// Recipients with no OpenPGP key: find or import one.
    FindPgpKey,
    /// An identity with no OpenPGP key: make one.
    MakePgpKey { address: String },
    /// A secret key about to be forgotten should be exported first.
    ExportPgpSecret { fingerprint: Fingerprint },
    /// Recipients with no S/MIME certificate: get theirs or import one.
    ImportCertificate,
    /// An identity with no S/MIME certificate: import its own.
    ImportOwnCertificate,
}
