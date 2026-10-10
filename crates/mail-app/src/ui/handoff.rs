//! The window's end of the handoff: reading what the running window is asked.
//!
//! The bus, the requests and the token are the application's own (`crate::handoff`), shared with
//! `mailo open` and the intent provider, which hand a conversation to the window and have no
//! window of their own. This is only the part that draws: it reads the queue on the thread that
//! draws and opens each conversation as a click on its row would.

use super::view::Shell;
use crate::handoff::Request;
use dioxus::prelude::*;

/// Read what the running window is asked, on the thread that draws: open each conversation in
/// this window (`shell`, as a click on its row would) and raise the window with the request's
/// activation token. Called once, from `App`; a window with no [`Requests`] reads nothing.
///
/// [`Requests`]: crate::handoff::Requests
#[cfg(not(any(target_os = "macos", windows)))]
pub(in crate::ui) fn use_handoff(shell: Signal<Shell>) {
    use crate::handoff::{ActivationToken, Requests};

    let Some(requests) = try_consume_context::<Requests>() else {
        return;
    };
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
                        // Said on stderr, which the session's journal keeps: the one trace of what
                        // a click did to a window with no other way to ask it.
                        eprintln!("opened conversation {thread}");
                        super::open_thread(&mut shell.write(), thread);
                        token
                    }
                    Request::Activate { token } => token,
                };
                if let Some(window) = &window {
                    window.focus_with_token(token.map(ActivationToken::into_string));
                }
            }
        }
    });
}

/// Where there is no session bus nothing is served, so there is nothing to read.
#[cfg(any(target_os = "macos", windows))]
pub(in crate::ui) fn use_handoff(_shell: Signal<Shell>) {
    let _: Option<Request> = None;
}
