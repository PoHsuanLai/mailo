//! What the window asks of whatever draws it: focus, a scroll, the clipboard.
//!
//! Every such request goes through [`Host`], as a typed [`Ask`], and nowhere else. On the
//! webview an ask is the script it has always been, evaluated in the page, word for word
//! ([`Ask::script`]). A renderer with no script engine answers the same asks its own way,
//! so no call site changes when the renderer does (Phase B's seam, `ui/host.rs`).
//!
//! Tests hand the window a [`Recorder`] as its `Host`: every ask is kept, in order, and none is
//! run, so a test asserts "the window asked to focus the find field" without a webview.

use dioxus::prelude::*;

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

/// A field that is focused, with its text selected, once it is drawn. Its script looks for it
/// for up to twenty frames, because the keystroke that opened it is handled before the render
/// that draws it reaches the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Drawn {
    /// The reader's find field, `.find .inp` (Ctrl F).
    FindField,
    /// The sidebar's folder name field, `.fold-edit input` (New folder, Rename).
    FolderName,
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

/// `(function focusFind(tries) {…})(20)`: the find field, retried for twenty frames.
const FOCUS_FIND: &str = "(function focusFind(tries) {\
    const field = document.querySelector('.find .inp');\
    if (field) { field.focus(); field.select(); }\
    else if (tries > 0) { requestAnimationFrame(() => focusFind(tries - 1)); }\
})(20)";

/// `(function focusName(tries) {…})(20)`: the folder name field, retried for twenty frames.
const FOCUS_NAME: &str = "(function focusName(tries) {\
    const input = document.querySelector('.fold-edit input');\
    if (input) { input.focus(); input.select(); }\
    else if (tries > 0) { requestAnimationFrame(() => focusName(tries - 1)); }\
})(20)";

impl Ask {
    /// The script the webview evaluates for this ask. These are the strings the window has
    /// always sent, unchanged; `host_tests` pins every one.
    pub(in crate::ui) fn script(&self) -> String {
        match self {
            Ask::FocusApp => "document.querySelector('.app')?.focus()".to_owned(),
            Ask::Focus {
                selector,
                when: When::Now,
            } => format!("document.querySelector('{selector}')?.focus()"),
            Ask::Focus {
                selector,
                when: When::NextFrame,
            } => {
                format!("requestAnimationFrame(()=>document.querySelector('{selector}')?.focus())")
            }
            Ask::Focus {
                selector,
                when: When::AfterTask,
            } => format!("setTimeout(() => document.querySelector('{selector}')?.focus())"),
            Ask::FocusAndSelect(Drawn::FindField) => FOCUS_FIND.to_owned(),
            Ask::FocusAndSelect(Drawn::FolderName) => FOCUS_NAME.to_owned(),
            Ask::ScrollIntoView(selector) => format!(
                "requestAnimationFrame(() => requestAnimationFrame(() => \
                 document.querySelector('{selector}')?.scrollIntoView({{ block: 'center' }})))"
            ),
            // Quoted by `serde_json`, like every string the window hands a script.
            Ask::Copy(text) => {
                let quoted = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned());
                format!("navigator.clipboard && navigator.clipboard.writeText({quoted})")
            }
        }
    }
}

/// Whatever draws the window, as a root context. Without one, the webview.
#[derive(Clone, Default)]
pub(in crate::ui) enum Host {
    /// The webview: each ask is its script, evaluated in the page.
    #[default]
    Webview,
    /// A test's recorder: each ask is kept and none is run.
    #[cfg(test)]
    Recording(Recorder),
}

impl Host {
    /// The host the window was handed, or the webview.
    fn current() -> Host {
        try_consume_context::<Host>().unwrap_or_default()
    }

    /// Hand `ask` to the current host.
    pub(in crate::ui) fn ask(ask: Ask) {
        match Host::current() {
            Host::Webview => {
                let _ = dioxus::document::eval(&ask.script());
            }
            #[cfg(test)]
            Host::Recording(recorder) => recorder.0.borrow_mut().push(ask),
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
