//! What an import did, in words: the command line prints it, the window shows it.

use mail_client::import::{Destination, Imported};

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
    if total.partial > 0 {
        out.push_str(&format!(
            "; {}",
            crate::ui::copy::status::import_without_attachments(total.partial)
        ));
    }
    if total.cut_off > 0 {
        out.push_str(&format!(
            "; {}",
            crate::ui::copy::status::import_cut_off(total.cut_off)
        ));
    }
    out.push('\n');
    out
}
