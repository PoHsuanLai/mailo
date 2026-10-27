//! Erasing Trash and Spam while the window runs (`mail_core::erase`): after the time the person
//! chose in Settings › General › Mailboxes, and, under "Delete deletes now", what Delete moved as
//! soon as the server holds it in Trash.
//!
//! Swept after any write (a sync's included, which is what lands a move in Trash) and once a
//! minute besides, so a day in Trash ends on time in a window left open with nothing arriving.
//! On a blocking thread, as the counts are. Nothing at all is read under the defaults, which keep
//! mail until it is erased by hand.

use crate::settings::MailSettings;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use std::time::Duration;

/// How often the time in Trash and Spam is looked at when nothing else moves.
const EVERY: Duration = Duration::from_secs(60);

/// Keep Trash and Spam erased as the settings say. A window with no settings (a test's) erases
/// nothing.
pub(super) fn use_eraser(revision: Signal<u64>, shell: Signal<Shell>) {
    let settings = use_hook(try_consume_context::<Signal<MailSettings>>);
    let mut tick = use_signal(|| 0_u64);
    use_future(move || async move {
        loop {
            tokio::time::sleep(EVERY).await;
            tick += 1;
        }
    });
    let _sweep = use_resource(move || {
        let _ = revision();
        let _ = tick();
        let policy = settings
            .map(|settings| settings.read().mailboxes.erasing())
            .unwrap_or_default();
        let client = crate::ui::live::client();
        let mut revision = revision;
        let mut shell = shell;
        async move {
            if policy.idle() {
                return;
            }
            let erased = tokio::task::spawn_blocking(move || {
                client.erasing().sweep(&policy, chrono::Utc::now())
            })
            .await;
            match erased {
                Ok(gone) if !gone.is_empty() => {
                    // An Undo of the Delete that put them there has nothing left to bring back.
                    shell.write().undo.forget_messages(&gone);
                    revision += 1;
                }
                Ok(_) => {}
                Err(why) => log::warn!("erasing Trash and Spam: {why}"),
            }
        }
    });
}

#[cfg(test)]
mod tests;
