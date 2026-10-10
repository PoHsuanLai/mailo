//! What an account that cannot sync is told to do about it.
//!
//! These are the words of [`crate::error::CoreError`] and of a sync pass's reasons, which name the
//! one command that supplies a credential; they live apart from [`super`] so that file holds no
//! command line.

use mail_domain::AuthPlan;
use porter_provider::Issuer;

/// What to tell someone whose account has no credential stored yet.
///
/// Per account, because the answer differs and getting it wrong is not a matter of tone. Every
/// account used to be told to set `MAILO_PASSWORD`. For a password account that is right. For a
/// Google one it is advice that cannot work — Google stopped accepting passwords for IMAP in May
/// 2022 — and following it means a failed sign-in against Google with a credential that was
/// never going to be accepted, which is the hazard this whole project has been careful about.
/// `mailo account add` already said the right thing; `mailo sync` contradicted it, and sync is
/// the command someone runs second.
///
/// `microsoft` carries `--microsoft` into the command, because the address alone does not
/// reproduce a managed-tenant account: that flag is exactly what the preset table cannot work
/// out, and a re-run without it finds no preset at all.
pub fn no_credential(address: &str, auth: &AuthPlan) -> String {
    match auth {
        AuthPlan::OAuth { issuer, .. } => {
            let flag = match issuer {
                Issuer::Microsoft => " --microsoft",
                _ => "",
            };
            format!(
                concat!(
                    "not signed in yet. This account uses OAuth ({issuer:?}), which needs a ",
                    "client id registered with the issuer — a password will not work. Run:\n",
                    "    MAILO_OAUTH_CLIENT_ID=… {secret}mailo account add {address}{flag}"
                ),
                issuer = issuer,
                address = address,
                flag = flag,
                // Google will not exchange a code without the application secret it issued.
                secret = match issuer {
                    Issuer::Google => "MAILO_OAUTH_CLIENT_SECRET=… ",
                    _ => "",
                },
            )
        }
        AuthPlan::Password { .. } => {
            format!("no credential stored. Run:\n    MAILO_PASSWORD=… mailo account add {address}")
        }
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
pub fn no_client_id(issuer: Issuer, address: &str) -> String {
    format!(
        concat!(
            "the sign-in has expired and no OAuth client id is configured ",
            "for {:?}. Re-run: MAILO_OAUTH_CLIENT_ID=… mailo account add {}",
        ),
        issuer, address
    )
}
