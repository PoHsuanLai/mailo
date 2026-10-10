//! `mailo export`: what it says as it goes and when it is done.

use super::SqliteStore;
use crate::said::export::said;
use mail_core::error::CoreError;
use mail_core::export::{self, Target};

/// `mailo export QUERY… --mbox FILE | --maildir DIR | --eml DIR`: what matches is counted on
/// stderr, then progress every hundred, then the answer.
pub fn run(store: &SqliteStore, query: &str, target: &Target) -> Result<String, CoreError> {
    let now = chrono::Utc::now();
    let chosen = export::select(store, query, now)?;
    eprintln!("{} message(s) match", chosen.len());
    let done = export::export(
        store,
        &crate::edge::environment(),
        &chosen,
        target,
        now,
        &mut |done| {
            eprintln!("  {} written", done.written);
        },
    )?;
    Ok(said(&done, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::export::Exported;

    #[test]
    fn it_says_how_many_were_written_and_why_the_rest_were_not() {
        let target = Target::Mbox("out.mbox".into());
        assert_eq!(
            said(
                &Exported {
                    written: 4,
                    ..Exported::default()
                },
                &target
            ),
            "4 message(s) written to out.mbox\n"
        );
        let skipped = said(
            &Exported {
                written: 0,
                absent: 1,
                partial: 2,
            },
            &target,
        );
        assert_eq!(
            skipped,
            "0 message(s) written to out.mbox\n\
             3 skipped: 1 with no body downloaded yet, 2 with attachments still on the \
             server. `mailo sync` downloads bodies; `mailo save` fetches an attachment.\n"
        );
    }
}
