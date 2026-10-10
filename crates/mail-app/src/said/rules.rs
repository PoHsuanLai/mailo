//! What pushing the rules to a server did, in words: the command line prints it, the window
//! shows it.

use mail_core::Pushed;
use mail_proto::sieve::{Deleted, SieveOutcome, VacationPlaced};
use std::fmt::Write as _;

/// What a push did, for a person.
pub fn said(address: &str, pushed: &Pushed) -> String {
    let mut out = String::new();
    match &pushed.outcome {
        SieveOutcome::Installed { displaced, .. } => {
            let _ = writeln!(
                out,
                "{address}: the server now runs {} rule(s){}",
                pushed.compiled.mapped.len(),
                match pushed.compiled.vacation {
                    VacationPlaced::Dated | VacationPlaced::Undated => " and the vacation reply",
                    _ => "",
                }
            );
            if let Some(theirs) = displaced {
                let _ = writeln!(
                    out,
                    "  {theirs:?} is no longer active; it is still on the server"
                );
            }
        }
        SieveOutcome::Refused { active, .. } => {
            let _ = writeln!(
                out,
                "{address}: nothing installed. The server runs a script called {active:?}, made \
                 elsewhere, and a server runs one script at a time. Merge its rules into \
                 `mailo rules`, then `mailo sieve push --replace-active`"
            );
        }
        SieveOutcome::Removed { deleted, .. } => {
            let _ = writeln!(
                out,
                "{address}: nothing here for the server to run{}",
                match deleted {
                    Deleted::Ours => "; this client's script is taken down",
                    Deleted::NothingThere => "",
                }
            );
        }
        SieveOutcome::Status { .. } => {}
    }
    for (name, why) in &pushed.compiled.local_only {
        let _ = writeln!(out, "  {name:?} runs in this client only: {why}");
    }
    match pushed.compiled.vacation {
        VacationPlaced::Undated => out.push_str(
            "  the server cannot test dates, so the reply is on until a push after its end \
             (`mailo sieve push`, or `vacation off`)\n",
        ),
        VacationPlaced::Outside => out.push_str(
            "  the reply is outside its dates and the server cannot test them: it is left out, \
             and a push during them puts it in\n",
        ),
        VacationPlaced::Unsupported => {
            out.push_str("  the server's Sieve has no vacation extension: no reply is sent\n");
        }
        VacationPlaced::Absent | VacationPlaced::Dated => {}
    }
    out
}
