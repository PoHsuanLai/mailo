//! The window's end of the handoff: reading what the running window is asked.
//!
//! The bus, the requests and the token are the application's own (`crate::handoff`), shared with
//! `mailo open` and the intent provider, which hand a conversation to the window and have no
//! window of their own. This is only the part that draws: it reads the queue on the thread that
//! draws and opens each conversation as a click on its row would.

use super::view::Shell;
use crate::handoff::Request;
use crate::ui::space::Handle;
use dioxus::prelude::*;

/// Read what the running window is asked, on the thread that draws: open each conversation in
/// this window (`shell`, as a click on its row would), in a Space that shows its account
/// (`spaces` switches to it), and raise the window with the request's activation token. Called
/// once, from `App`; a window with no [`Requests`] reads nothing.
///
/// [`Requests`]: crate::handoff::Requests
#[cfg(not(windows))]
pub(in crate::ui) fn use_handoff(shell: Signal<Shell>, spaces: Handle) {
    use crate::handoff::{ActivationToken, Requests};

    let Some(requests) = try_consume_context::<Requests>() else {
        return;
    };
    // The Dock icon clicked again reaches the queue through the application delegate. It can only
    // be set on the main thread once winit has made `NSApplication`, which is by the time `App`
    // first draws. (URLs the system opens are heard earlier, in `serve`.)
    #[cfg(target_os = "macos")]
    use_hook({
        let requests = requests.clone();
        move || {
            if !crate::handoff::hear_reopen(&requests) {
                log::warn!("the Dock icon is not heard: no application delegate");
            }
        }
    });
    let window = ds_blitz::use_window_handle();
    use_future(move || {
        let requests = requests.clone();
        let window = window.clone();
        let mut shell = shell;
        async move {
            let Some(mut queue) = requests.take() else {
                return;
            };
            while let Some(request) = queue.recv().await {
                let token = match request {
                    Request::Thread { thread, token } => {
                        // Logged: the one trace of what a click did to a window with no other way
                        // to ask it.
                        log::info!(target: crate::ui::TRACE, "opened conversation {thread}");
                        arrive(spaces, shell, &super::Start::Thread(thread));
                        token
                    }
                    Request::Reply { thread, token } => {
                        log::info!(target: crate::ui::TRACE, "asked for a reply to {thread}");
                        let client = crate::ui::live::client();
                        // From the account the conversation is in, with its newest message
                        // answered, as the Reply key does.
                        let writes = super::ops::Writes::On(
                            thread,
                            super::ops::Composes::Reply(mail_domain::ReplyScope::Sender),
                        );
                        let _ = super::ops::open_composer(&client, shell, writes);
                        token
                    }
                    Request::Inbox { account, token } => {
                        log::info!(target: crate::ui::TRACE, "opened the inbox of {account}");
                        arrive(spaces, shell, &super::Start::InboxOf(account));
                        token
                    }
                    Request::Mailto { link, token } => {
                        // The draft is saved first, as at a `mailto:` start, then the composer
                        // opens on it.
                        let client = crate::ui::live::client();
                        let saved = super::start_mailto(&client, &link, chrono::Utc::now());
                        match saved {
                            Ok(super::Start::Compose(id)) => {
                                if let Err(why) = shell.write().compose_id(&client, id) {
                                    log::warn!("compose: {why}");
                                }
                            }
                            Ok(_) => {}
                            Err(why) => log::warn!("{why}"),
                        }
                        token
                    }
                    Request::Activate { token } => {
                        log::info!(target: crate::ui::TRACE, "asked to raise the window");
                        token
                    }
                };
                if let Some(window) = &window {
                    window.focus_with_token(token.map(ActivationToken::into_string));
                }
            }
        }
    });
}

/// Show what a click asked for (`start`): first in the Space [`super::start::space_for`] chooses,
/// switched to as ⌘1-9 would (where the window was is remembered for the Space it leaves), then
/// the conversation or the inbox there. No switch when the window is already in a Space that
/// shows the account, when none does, or while a Space's part is open (quire refuses one then);
/// it is then shown in the Space on screen.
#[cfg(not(windows))]
fn arrive(spaces: Handle, mut shell: Signal<Shell>, start: &super::Start) {
    let client = crate::ui::live::client();
    let (chosen, current) = {
        let list = spaces.spaces();
        let list = list.peek();
        (
            super::start::space_for(&list, &client, start),
            list.current().id,
        )
    };
    if let Some(id) = chosen.filter(|id| *id != current) {
        // `None` while a Space's part is open: the click is then shown where the window is.
        let _ = spaces.switch(id);
    }
    super::start::show(&mut shell.write(), start);
}

/// Where there is no bus and no socket nothing is served, so there is nothing to read.
#[cfg(windows)]
pub(in crate::ui) fn use_handoff(_shell: Signal<Shell>, _spaces: Handle) {
    let _: Option<Request> = None;
}
