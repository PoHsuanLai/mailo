//! What importing contacts did, in words: the command line prints it, the window shows it.

use mail_core::contacts::Imported;
use std::fmt::Write as _;

/// What reading a `.vcf` file says.
pub fn imported(done: &Imported) -> String {
    let mut out = format!(
        "imported {} addresses from {} cards\n",
        done.addresses, done.cards
    );
    if done.groups > 0 {
        let _ = writeln!(
            out,
            "imported {} {}",
            done.groups,
            if done.groups == 1 { "group" } else { "groups" }
        );
    }
    if done.empty > 0 {
        let _ = writeln!(
            out,
            "{} cards had no email address and were skipped",
            done.empty
        );
    }
    out
}
