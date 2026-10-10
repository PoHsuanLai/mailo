//! What an account that cannot sync is told about it.
//!
//! These are the words of [`crate::error::CoreError`] and of a sync pass's reasons. They say what
//! is wrong; what to do about it is a [`crate::Remedy`], which the front end words, so that this
//! file holds no command line.

use mail_domain::AuthPlan;
use porter_provider::Issuer;

/// What to tell someone whose account has no credential stored yet.
///
/// Per account, because the answer differs and getting it wrong is not a matter of tone. Every
/// account used to be told to set `MAILO_PASSWORD`. For a password account that is right. For a
/// Google one it is advice that cannot work — Google stopped accepting passwords for IMAP in May
/// 2022 — and following it means a failed sign-in against Google with a credential that was
/// never going to be accepted, which is the hazard this whole project has been careful about.
/// So an OAuth account says it needs a client id, and a password account only that none is stored;
/// the steps for either are [`crate::Remedy::SignIn`]'s, worded by the front end.
pub fn no_credential(address: &str, auth: &AuthPlan) -> String {
    match auth {
        AuthPlan::OAuth { issuer, .. } => format!(
            "not signed in yet. This account uses OAuth ({issuer:?}), which needs a client id \
             registered with the issuer — a password will not work"
        ),
        AuthPlan::Password { .. } => "no credential stored".to_owned(),
        // Not a credential of ours to be missing: the grant is what is wanted.
        AuthPlan::Granted { .. } => format!(
            "the desktop's account service does not let Mail use {address} (any more). Allow it \
             again in Add Account"
        ),
    }
}

/// What an OAuth account that has expired is told when this installation has no client id for
/// its issuer, and so cannot renew it.
///
/// Beside [`no_credential`], which says the same kind of thing about an account that never
/// signed in: the remedy is the same, and so is who needs to read it.
pub fn no_client_id(issuer: Issuer) -> String {
    format!("the sign-in has expired and no OAuth client id is configured for {issuer:?}")
}
