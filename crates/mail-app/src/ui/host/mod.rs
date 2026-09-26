//! What the window asks of whatever draws it: focus, a scroll, the clipboard.
//!
//! Every such request goes through [`Host`], as a typed [`Ask`], and nowhere else. There is no
//! script engine: the launched window names [`Host::Native`] (`native.rs`), which answers each
//! ask in the Blitz document.
//!
//! Tests hand the window a [`Recorder`] as its `Host`: every ask is kept, in order, and none is
//! run, so a test asserts "the window asked to focus the find field" without a window.

use dioxus::prelude::*;

mod native;

/// When a field is focused, relative to the render that draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum When {
    /// Now: the element is already in the page.
    Now,
    /// After the next frame, once the render that opened it has reached the page.
    NextFrame,
    /// After the task that is running, once a menu's pick has been drawn.
    AfterTask,
}

/// A field that is focused, with its text selected, once it is drawn. The host looks for it for
/// up to twenty frames, because the keystroke that opened it is handled before the render that
/// draws it reaches the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Drawn {
    /// The reader's find field, `.find input` (Ctrl F).
    FindField,
    /// The sidebar's new folder name field, `.fold-edit input`.
    FolderName,
}

impl Drawn {
    /// The field, as the selector the host looks for.
    pub(in crate::ui) fn selector(self) -> &'static str {
        match self {
            Drawn::FindField => ".find input",
            Drawn::FolderName => ".fold-edit input",
        }
    }
}

/// One thing the window asks of its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Ask {
    /// Give the keyboard back to the window, `.app`, so the next letter is a shortcut again.
    FocusApp,
    /// Focus the first element `selector` matches, `when` it is there.
    Focus { selector: &'static str, when: When },
    /// Focus a field once it is drawn, and select what is in it.
    FocusAndSelect(Drawn),
    /// Bring the first element `selector` matches into the middle of its scroller, after the
    /// render that moved it.
    ScrollIntoView(&'static str),
    /// Put text on the clipboard.
    Copy(String),
}

/// Whatever draws the window, as a root context.
///
/// The launched window provides [`Host::Native`] ([`use_window_host`]). A test that renders
/// `App` alone provides none, and its asks go nowhere.
#[derive(Clone, Default)]
pub(in crate::ui) enum Host {
    /// No host: a window drawn without one, as a test draws it. Each ask is dropped.
    #[default]
    Absent,
    /// Blitz: each ask is answered in the document, with no script.
    Native(native::Blitz),
    /// A test's recorder: each ask is kept and none is run.
    #[cfg(test)]
    Recording(Recorder),
}

/// Name the launched window's host, once, above `App`: Blitz's.
pub(in crate::ui) fn use_window_host() {
    use_context_provider(|| Host::Native(native::Blitz::new()));
}

impl Host {
    /// The host the window was handed, or none.
    fn current() -> Host {
        try_consume_context::<Host>().unwrap_or_default()
    }

    /// Hand `ask` to the current host.
    pub(in crate::ui) fn ask(ask: Ask) {
        match Host::current() {
            Host::Absent => {}
            Host::Native(blitz) => blitz.ask(ask),
            #[cfg(test)]
            Host::Recording(recorder) => recorder.0.borrow_mut().push(ask),
        }
    }

    /// `.app`, the element the keyboard lands on, has mounted. Blitz keeps its handle, to give
    /// it the keyboard back later, and gives it the keyboard now.
    pub(in crate::ui) fn app_mounted(event: MountedEvent) {
        if let Host::Native(blitz) = Host::current() {
            blitz.mounted(event.data());
        }
    }

    /// Give the keyboard back to the window, `.app`.
    pub(in crate::ui) fn focus_app() {
        Host::ask(Ask::FocusApp);
    }

    /// Focus the element `selector` matches, now.
    pub(in crate::ui) fn focus(selector: &'static str) {
        Host::ask(Ask::Focus {
            selector,
            when: When::Now,
        });
    }

    /// Focus the element `selector` matches after the next frame: a sheet's field, once the
    /// render that opened the sheet is in the page.
    pub(in crate::ui) fn focus_next_frame(selector: &'static str) {
        Host::ask(Ask::Focus {
            selector,
            when: When::NextFrame,
        });
    }

    /// Focus the element `selector` matches once the running task is done: a field a menu's
    /// pick opened.
    pub(in crate::ui) fn focus_after_task(selector: &'static str) {
        Host::ask(Ask::Focus {
            selector,
            when: When::AfterTask,
        });
    }

    /// Focus `field` once it is drawn, and select its text.
    pub(in crate::ui) fn focus_and_select(field: Drawn) {
        Host::ask(Ask::FocusAndSelect(field));
    }

    /// Scroll the element `selector` matches into the middle of its scroller.
    pub(in crate::ui) fn scroll_into_view(selector: &'static str) {
        Host::ask(Ask::ScrollIntoView(selector));
    }

    /// Put `text` on the clipboard.
    pub(in crate::ui) fn copy(text: &str) {
        Host::ask(Ask::Copy(text.to_owned()));
    }
}

/// A host that keeps every ask, in order, and runs none: the test double.
#[cfg(test)]
#[derive(Clone, Default)]
pub(in crate::ui) struct Recorder(std::rc::Rc<std::cell::RefCell<Vec<Ask>>>);

#[cfg(test)]
impl Recorder {
    /// The host to give the window as its root context.
    pub(in crate::ui) fn host(&self) -> Host {
        Host::Recording(self.clone())
    }

    /// Everything asked so far.
    pub(in crate::ui) fn asked(&self) -> Vec<Ask> {
        self.0.borrow().clone()
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod host_tests;
