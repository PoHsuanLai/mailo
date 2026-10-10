//! `icons refresh`: one line per provider, what landed.

use mail_core::provider::Provider;
use mail_core::provider::icon::{IconError, file_stem};
use std::fmt::Write as _;

/// What `mailo icons refresh` prints. Failures are a line here; the caller logs them.
pub(super) fn report(results: &[(Provider, Result<usize, IconError>)]) -> String {
    let mut out = String::new();
    for (provider, result) in results {
        let stem = file_stem(*provider);
        match result {
            Ok(bytes) => {
                let _ = writeln!(out, "{stem}: wrote {bytes} bytes");
            }
            Err(IconError::Unmapped) => {
                let _ = writeln!(out, "{stem}: letters only");
            }
            Err(_) => {
                let _ = writeln!(out, "{stem}: not updated");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refresh_report_names_what_landed() {
        let lines = report(&[
            (Provider::Google, Ok(12)),
            (Provider::Imap, Err(IconError::Unmapped)),
            (Provider::Yahoo, Err(IconError::NotHttps)),
        ]);
        assert_eq!(
            lines,
            "google: wrote 12 bytes\nimap: letters only\nyahoo: not updated\n"
        );
    }
}
