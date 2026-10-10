//! A conversation in a window of its own: "Open in new window" in the reader's menu and on a
//! row's context menu, and Shift+Enter.
//!
//! The window is quire's (`ds_blitz::open_window_with`): a VirtualDom of its own with the same
//! root contexts `launch` gave the first window, so it reads the same store and the same
//! [`super::revisions::Revisions`], which is how either window sees what the other did. It shows
//! the reader and nothing else (`root.rs`). Its consent to remote images starts empty: it is its
//! own reader, holding its own grant (`ui/original/consent.rs`).
//!
//! The call goes through [`Windows`] when the window was given one, which is how a test sees
//! what would open: quire's harness has no event loop to open a window on
//! (`OpenWindowError::NoHost`).

mod note;
mod root;

pub use root::{MessageOpen, message_root};

use dioxus::prelude::*;
use ds::prelude::Icon;
use ds_blitz::{WindowHandle, WindowLife, WindowSpec};
use mail_domain::ThreadId;
use mail_store::{SqliteStore, Store};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// A conversation to open in a window of its own, and the window's title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub thread: ThreadId,
    /// The conversation's subject.
    pub title: String,
}

/// Whatever opens a conversation's window: quire's event loop in the launched window, a recorder
/// in a test.
pub trait OpenWindow: Send + Sync + 'static {
    /// Open `ask`'s conversation in a window of its own.
    fn open(&self, ask: Ask);
}

/// Where "Open in new window" goes, as a root context. Without one it is quire's
/// `open_window_with`. Cloning shares it.
#[derive(Clone)]
pub struct Windows(pub Arc<dyn OpenWindow>);

impl std::fmt::Debug for Windows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Windows")
    }
}

/// The windows this window has opened, by conversation: opening one already open raises it.
/// Held by the window that opened them; a `WindowHandle` is quire's and stays on this thread.
#[derive(Clone, Default)]
pub(in crate::ui) struct Opened(Rc<RefCell<HashMap<ThreadId, WindowHandle>>>);

/// Keep the windows this one opens. Called once, from `App`.
pub(in crate::ui) fn use_opened() {
    use_context_provider(Opened::default);
}

/// The size a conversation's window opens at, in logical pixels.
const SIZE: (u32, u32) = (760, 720);

/// Open `thread` in a window of its own, titled with its subject. A conversation the store no
/// longer has opens nothing.
pub(in crate::ui) fn open_in_window(thread: ThreadId) {
    let store = consume_context::<Arc<SqliteStore>>();
    let Ok(loaded) = store.thread(thread) else {
        return;
    };
    let ask = Ask {
        thread,
        title: title(&loaded.summary.subject),
    };
    match try_consume_context::<Windows>() {
        Some(windows) => windows.0.open(ask),
        None => quire(ask),
    }
}

/// A window's title for a conversation whose subject is `subject`.
pub(in crate::ui) fn title(subject: &str) -> String {
    match subject.trim() {
        "" => "(no subject)".to_owned(),
        said => said.to_owned(),
    }
}

/// Whether a window opened earlier is one to raise rather than open again. The Settings and
/// Add Account windows ask the same question.
pub(in crate::ui) fn raise(life: Option<WindowLife>) -> bool {
    matches!(life, Some(WindowLife::Opening | WindowLife::Open))
}

/// Ask quire's event loop for the window, or raise the one already open for it.
fn quire(ask: Ask) {
    let opened = try_consume_context::<Opened>().unwrap_or_default();
    let existing = opened.0.borrow().get(&ask.thread).cloned();
    if let Some(handle) = existing.filter(|handle| raise(Some(handle.life()))) {
        handle.focus();
        return;
    }
    let spec = WindowSpec::new(ask.title, ds_blitz::WindowSize::new(SIZE.0, SIZE.1));
    match ds_blitz::open_window_with(
        spec,
        root::MessageWindow,
        root::MessageWindowProps { thread: ask.thread },
    ) {
        Ok(handle) => {
            opened.0.borrow_mut().insert(ask.thread, handle);
        }
        Err(why) => super::motion::tell(
            format!("Could not open a window: {why}"),
            super::motion::Follow::Nothing,
        ),
    }
}

/// The menu row that opens a conversation in its own window, with the key that does it.
pub(in crate::ui) fn menu_item() -> super::menu::MenuItem {
    super::menu::MenuItem {
        key: OPEN_KEY.to_owned(),
        tile: super::menu::Tile::Icon(Icon::Window),
        name: "Open in new window".to_owned(),
        help: None,
        right: super::menu::Right::Shortcut(SHORTCUT.to_owned()),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

/// The menu row's key.
pub(in crate::ui) const OPEN_KEY: &str = "open-window";

/// The key that opens the focused conversation in its own window, as the menus write it.
const SHORTCUT: &str = "⇧ Enter";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_still_there_is_raised_and_a_closed_one_opened_again() {
        const CASES: &[(Option<WindowLife>, bool)] = &[
            (None, false),
            (Some(WindowLife::Opening), true),
            (Some(WindowLife::Open), true),
            (Some(WindowLife::Closed), false),
        ];
        for (life, raised) in CASES {
            assert_eq!(raise(*life), *raised, "{life:?}");
        }
    }

    #[test]
    fn the_title_is_the_subject_and_says_when_there_is_none() {
        const CASES: &[(&str, &str)] = &[
            ("Lunch on Thursday", "Lunch on Thursday"),
            ("  Re: plans \t", "Re: plans"),
            ("", "(no subject)"),
            ("   ", "(no subject)"),
        ];
        for (subject, want) in CASES {
            assert_eq!(title(subject), *want, "{subject:?}");
        }
    }
}
