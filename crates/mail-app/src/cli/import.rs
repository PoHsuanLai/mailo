//! `mailo import`: what it says as it goes and when it is done.

use super::SqliteStore;
use mail_core::import::{self, Destination, Imported};
use std::sync::Arc;

/// What an import says when it is done.
pub fn said(total: &Imported, into: &Destination) -> String {
    let place = match into {
        Destination::Local => "kept in local folders".to_owned(),
        Destination::Mailbox { account, folder } => format!("queued for {folder} on {account}"),
    };
    let mut out = format!(
        "{} message(s) read; {} {place}; {} already there",
        total.read, total.added, total.already
    );
    if total.unreadable > 0 {
        out.push_str(&format!("; {} not a message, skipped", total.unreadable));
    }
    out.push('\n');
    out
}

/// The line printed to stderr after each batch.
pub fn progress(so_far: &Imported) -> String {
    format!("  {} read, {} new", so_far.read, so_far.added)
}

/// `mailo import`: keep the mail here, or queue it for a mailbox and send it now.
pub fn run(
    store: &Arc<SqliteStore>,
    path: &std::path::Path,
    into: &Destination,
) -> Result<String, String> {
    let now = chrono::Utc::now();
    let source = import::detect(path)?;
    let mut tell = |so_far: &Imported| eprintln!("{}", progress(so_far));
    match into {
        Destination::Local => {
            let total = import::into_local(store, &source, now, &mut tell)?;
            Ok(said(&total, into))
        }
        Destination::Mailbox { account, folder } => {
            let (id, total) =
                import::queue_uploads(store, account, folder, &source, now, &mut tell)?;
            let mut out = said(&total, into);
            // Sent now, so the user sees it go; whatever fails stays queued for the next sync.
            let mail = crate::edge::mail(store);
            let report = crate::edge::block_on(mail.sync().drain(id))?;
            out.push_str(&format!("{} uploaded\n", report.appended));
            if report.still_queued > 0 {
                out.push_str(&format!(
                    "{} still queued; `mailo sync` tries again\n",
                    report.still_queued
                ));
            }
            for note in &report.needs_attention {
                out.push_str(&format!("  needs attention: {note}\n"));
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_says_what_was_read_kept_and_skipped() {
        let total = Imported {
            read: 5,
            added: 3,
            already: 1,
            unreadable: 1,
        };
        assert_eq!(
            said(&total, &Destination::Local),
            "5 message(s) read; 3 kept in local folders; 1 already there; 1 not a message, \
             skipped\n"
        );
        let uploaded = Destination::Mailbox {
            account: "me@example.test".to_owned(),
            folder: "Archive".to_owned(),
        };
        assert_eq!(
            said(
                &Imported {
                    unreadable: 0,
                    ..total
                },
                &uploaded
            ),
            "5 message(s) read; 3 queued for Archive on me@example.test; 1 already there\n"
        );
        assert_eq!(progress(&total), "  5 read, 3 new");
    }
}
