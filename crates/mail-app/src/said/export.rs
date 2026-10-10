//! What an export did, in words: the command line prints it, the window shows it.

use mail_core::export::{Exported, Target};

/// What an export says when it is done.
pub fn said(done: &Exported, target: &Target) -> String {
    let place = match target {
        Target::Mbox(path) | Target::Maildir(path) | Target::Eml(path) => path.display(),
    };
    let mut out = format!("{} message(s) written to {place}\n", done.written);
    let skipped = done.absent + done.partial;
    if skipped > 0 {
        out.push_str(&format!(
            "{skipped} skipped: {} with no body downloaded yet, {} with attachments still on \
             the server. `mailo sync` downloads bodies; `mailo save` fetches an attachment.\n",
            done.absent, done.partial
        ));
    }
    out
}
