//! The window's unread count, handed to the launcher (`mail_os::launcher`).
//!
//! Counted when the store moves (any write, a sync's included) or the Space changes, on a
//! blocking thread, and handed on only when the number changed, so a write that leaves it alone
//! sends the dock nothing.

use dioxus::prelude::*;
use mail_os::launcher::{self, Launcher, Unread};
use mail_ui_kit::view::Shell;

/// Keep the launcher's count. Does nothing in a window with no [`Launcher`], which is every test
/// that does not hand it a recorder.
pub(super) fn use_launcher_count(shell: Signal<Shell>) {
    let target = use_hook(try_consume_context::<Launcher>);
    let wanted = target.is_some();
    // A memo, so a keystroke (a shell change) does not recount; only a new scope does.
    let scope = use_memo(move || shell.read().scope.clone());
    let counted: Resource<Option<Unread>> = use_resource(move || {
        let _ = mail_ui_kit::live::moved_for(
            mail_client::Topics::SUMMARIES | mail_client::Topics::FOLDERS,
        );
        let scope = scope();
        let client = mail_ui_kit::live::client();
        async move {
            if !wanted {
                return None;
            }
            tokio::task::spawn_blocking(move || {
                let places = mail_ui_model::settings::badge_places();
                let stored = client.stored();
                let followed = match places {
                    mail_client::notify::Places::Inbox => Vec::new(),
                    mail_client::notify::Places::Followed => stored.followed_folders().ok()?,
                };
                let filter = launcher::filter_over(&followed, &scope, places);
                stored
                    .count(&filter, chrono::Utc::now())
                    .map(launcher::Unread)
                    .ok()
            })
            .await
            .ok()
            .flatten()
        }
    });
    let mut shown = use_signal(|| None::<Unread>);
    use_effect(move || {
        let Some(Some(unread)) = *counted.read() else {
            return;
        };
        if *shown.peek() == Some(unread) {
            return;
        }
        shown.set(Some(unread));
        if let Some(Launcher(badge)) = &target {
            badge.show(unread);
        }
    });
}
