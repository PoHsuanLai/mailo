//! What a process was started with, read once by the program and handed to the library.
//!
//! A library that reads `std::env` itself answers differently depending on who started the
//! process, and a test can only change that by writing the environment while other threads read
//! it. So nothing in this crate reads a variable: the program (`mail-app`'s `main`) builds an
//! [`Environment`] from the one place it can ([`Environment::from_lookup`]), and [`crate::Mail`]
//! carries it to the operations that take a password, a token, a client id or a hostname from
//! it. A test builds the value it wants.

use std::ffi::OsString;
use std::fmt;

/// The variables mailo reads, parsed. Every field is "not given" until the program gives it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Environment {
    /// `MAILO_PASSWORD`: the password `account add` and `contacts sync --user` keep in the
    /// keyring. As given: an empty one is no password, which the readers decide.
    pub password: Option<String>,
    /// `MAILO_JMAP_TOKEN`: a bearer token that signs in a JMAP account where a password would.
    /// Not given when empty.
    pub jmap_token: Option<String>,
    /// `MAILO_OAUTH_CLIENT_ID`: the OAuth client to sign in with. Not given when empty.
    pub oauth_client_id: Option<String>,
    /// `MAILO_OAUTH_CLIENT_SECRET`: the secret Google issues with a client. Not given when empty.
    pub oauth_client_secret: Option<String>,
    /// `MAILO_PGP_PASSPHRASE`: the passphrase of an OpenPGP key, for scripts.
    pub pgp_passphrase: Option<OsString>,
    /// `MAILO_SMIME_PASSWORD`: the password of a PKCS#12 file, for scripts.
    pub smime_password: Option<OsString>,
    /// This machine's name, from `HOSTNAME`, else Windows's `COMPUTERNAME`.
    pub hostname: Option<String>,
}

impl Environment {
    /// The value of each variable `get` finds. The program passes
    /// `|name| std::env::var_os(name)`; a test passes a table.
    pub fn from_lookup(get: impl Fn(&str) -> Option<OsString>) -> Self {
        let text = |name: &str| get(name).and_then(|value| value.into_string().ok());
        let given = |name: &str| text(name).filter(|value| !value.is_empty());
        Self {
            password: text("MAILO_PASSWORD"),
            jmap_token: given("MAILO_JMAP_TOKEN"),
            oauth_client_id: given("MAILO_OAUTH_CLIENT_ID"),
            oauth_client_secret: given("MAILO_OAUTH_CLIENT_SECRET"),
            pgp_passphrase: get("MAILO_PGP_PASSPHRASE"),
            smime_password: get("MAILO_SMIME_PASSWORD"),
            hostname: text("HOSTNAME").or_else(|| text("COMPUTERNAME")),
        }
    }
}

// By hand: most of these are secrets, and a stray `{:?}` must not print them.
impl fmt::Debug for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let said = |given: bool| if given { "<given>" } else { "<not given>" };
        f.debug_struct("Environment")
            .field("password", &said(self.password.is_some()))
            .field("jmap_token", &said(self.jmap_token.is_some()))
            .field("oauth_client_id", &self.oauth_client_id)
            .field(
                "oauth_client_secret",
                &said(self.oauth_client_secret.is_some()),
            )
            .field("pgp_passphrase", &said(self.pgp_passphrase.is_some()))
            .field("smime_password", &said(self.smime_password.is_some()))
            .field("hostname", &self.hostname)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn nothing_given_is_nothing() {
        assert_eq!(Environment::from_lookup(|_| None), Environment::default());
    }

    #[test]
    fn each_variable_lands_in_its_field() {
        let env = Environment::from_lookup(table(&[
            ("MAILO_PASSWORD", "hunter2"),
            ("MAILO_JMAP_TOKEN", "tok"),
            ("MAILO_OAUTH_CLIENT_ID", "client"),
            ("MAILO_OAUTH_CLIENT_SECRET", "shh"),
            ("MAILO_PGP_PASSPHRASE", "pgp"),
            ("MAILO_SMIME_PASSWORD", "smime"),
            ("HOSTNAME", "box"),
        ]));
        assert_eq!(env.password.as_deref(), Some("hunter2"));
        assert_eq!(env.jmap_token.as_deref(), Some("tok"));
        assert_eq!(env.oauth_client_id.as_deref(), Some("client"));
        assert_eq!(env.oauth_client_secret.as_deref(), Some("shh"));
        assert_eq!(env.pgp_passphrase, Some(OsString::from("pgp")));
        assert_eq!(env.smime_password, Some(OsString::from("smime")));
        assert_eq!(env.hostname.as_deref(), Some("box"));
    }

    #[test]
    fn an_empty_token_or_client_is_not_given_but_an_empty_password_is_kept_as_typed() {
        let env = Environment::from_lookup(table(&[
            ("MAILO_PASSWORD", ""),
            ("MAILO_JMAP_TOKEN", ""),
            ("MAILO_OAUTH_CLIENT_ID", ""),
            ("MAILO_OAUTH_CLIENT_SECRET", ""),
        ]));
        assert_eq!(env.password.as_deref(), Some(""));
        assert_eq!(env.jmap_token, None);
        assert_eq!(env.oauth_client_id, None);
        assert_eq!(env.oauth_client_secret, None);
    }

    #[test]
    fn the_hostname_falls_back_to_the_name_windows_gives_it() {
        let env = Environment::from_lookup(table(&[("COMPUTERNAME", "desk")]));
        assert_eq!(env.hostname.as_deref(), Some("desk"));
        let both =
            Environment::from_lookup(table(&[("HOSTNAME", "box"), ("COMPUTERNAME", "desk")]));
        assert_eq!(both.hostname.as_deref(), Some("box"));
    }

    #[test]
    fn debug_does_not_print_a_secret() {
        let env = Environment::from_lookup(table(&[
            ("MAILO_PASSWORD", "hunter2"),
            ("MAILO_JMAP_TOKEN", "tok-secret"),
            ("MAILO_PGP_PASSPHRASE", "pgp-secret"),
        ]));
        let shown = format!("{env:?}");
        for secret in ["hunter2", "tok-secret", "pgp-secret"] {
            assert!(!shown.contains(secret), "{shown}");
        }
    }
}
