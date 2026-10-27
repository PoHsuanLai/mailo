//! The window erases what Delete moved under "deletes now", and nothing under the defaults.

use super::use_eraser;
use crate::settings::{DeleteMeans, MailSettings};
use crate::ui::fixtures::{
    acct_account, dispatching, inbox_query, realistic, rebuild_into, settle_for,
};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_core::Store;
use mail_domain::*;
use std::time::Duration;

#[component]
fn Settings(delete: DeleteMeans) -> Element {
    use_hook(|| {
        let mut settings = MailSettings::default();
        settings.mailboxes.delete = delete;
        provide_context(Signal::new(settings));
    });
    rsx! { Erasing {} }
}

#[component]
fn Erasing() -> Element {
    let shell = use_signal(Shell::default);
    use_eraser(shell);
    rsx! {}
}

/// Delete moves a conversation to Trash; the window erases it only when Delete means "deletes
/// now".
#[tokio::test]
async fn delete_now_is_erased_by_the_window_and_trash_is_kept() {
    for (delete, kept) in [(DeleteMeans::Trash, true), (DeleteMeans::Now, false)] {
        let (store, _dir) = realistic();
        let thread = store
            .threads(&inbox_query(), chrono::Utc::now())
            .unwrap()
            .items[0]
            .id;
        let messages = store.thread(thread).unwrap().messages;
        // Delete's move, as the server has already taken it: nothing of it waits to be sent.
        let changes = messages
            .iter()
            .map(|m| Change::MessageMailbox(*m, MailboxRole::Trash))
            .collect();
        store
            .apply(
                acct_account(),
                &Patch {
                    id: ChangeId::generate(),
                    changes,
                },
            )
            .unwrap();
        if delete == DeleteMeans::Now {
            store.erase_soon(&messages, chrono::Utc::now()).unwrap();
        }
        dispatching();
        let mut dom = VirtualDom::new_with_props(Settings, SettingsProps { delete })
            .with_root_context(crate::ui::fixtures::client(&store));
        let mut seen = rebuild_into(&mut dom);
        settle_for(&mut dom, &mut seen, Duration::from_millis(400)).await;
        assert_eq!(
            messages.iter().all(|m| store.message(*m).is_ok()),
            kept,
            "{delete:?}"
        );
    }
}
