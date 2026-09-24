//! A password on its way to the keyring, and nowhere else.
//!
//! Held only between the moment it is typed (or read from `MAILO_PASSWORD`) and the moment
//! [`crate::account::add_with_password`] hands it to the keyring. It is not `Clone`, so it
//! cannot quietly be copied into a second place; its `Debug` is written by hand, so a stray
//! `{:?}` in a log line, a panic message or a test failure cannot print it; there is no
//! `Display` and no `serde`, so it cannot be shown or saved by accident; and its bytes are
//! overwritten when it is dropped.

use std::fmt;
use zeroize::Zeroize as _;

/// A password. See the module documentation for what it refuses to do.
#[derive(Default)]
pub struct Password(String);

impl Password {
    /// Take ownership of `typed`. The caller's copy is moved in, not duplicated.
    pub fn new(typed: String) -> Password {
        Password(typed)
    }

    /// Whether nothing was typed. An empty password is no password.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The text itself, for the one place that must have it: the credential put in the keyring.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() {
            "Password(<empty>)"
        } else {
            "Password(<redacted>)"
        })
    }
}

#[cfg(test)]
#[path = "password_tests.rs"]
mod tests;
