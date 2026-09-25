//! The window's asks answered on Blitz (`native`), where there is no script to evaluate.
//!
//! - The window itself, `.app`: its mounted handle is kept as it mounts, focused then so the
//!   keyboard has somewhere to land from the first key, and given the keyboard back through
//!   `ds::focus_soon`, which waits out a busy document. After a click on nothing focusable the
//!   keyboard stays on `.app` by itself: `ds_native::launch`'s `FocusFallback::Ancestor`. When
//!   what had the keyboard goes away (a menu closed on Escape), [`Blitz::hand_back`] gives it to
//!   `.app` through quire's own click-focus `restore`.
//! - A field or element named by selector: `ds::focus_by_selector`, which waits up to twenty
//!   frames for the element to be drawn, as the webview's scripts did, and tells a `TextInput`
//!   it found its `onfocus` once.
//! - A scroll into view: the element is found in the Blitz document reached through `.app`'s
//!   handle and scrolled, retried a frame later while the document is busy or the element is
//!   not drawn yet.
//! - The clipboard: `ds_native::clipboard::write_text`.

use super::Ask;
use blitz_dom::{ScrollBehavior, ScrollLogicalPosition};
use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use ds::Select;
use std::cell::RefCell;
use std::rc::Rc;

/// Frames a scroll waits for its element to be drawn, as the webview's scripts did.
const TRIES: usize = 20;

/// Blitz's side of the seam: `.app`'s handle once it has mounted, and the scope the work runs in.
#[derive(Clone)]
pub(in crate::ui) struct Blitz {
    app: Rc<RefCell<Option<Rc<MountedData>>>>,
    /// The window's shell, which outlives every panel. An ask is often made by a panel that
    /// is closing (Escape in the find field, a pick in the palette); a task of that panel's
    /// would be dropped with it before it ran, so every ask's task is the shell's.
    owner: ScopeId,
}

/// One try at a scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tried {
    Done,
    /// The document is being written, or the element is not drawn yet: try next frame.
    Later,
    /// It will never work: not a Blitz document, or not a selector Blitz reads.
    Never,
}

impl Blitz {
    /// The host for the window whose shell is the calling scope.
    pub(in crate::ui) fn new() -> Blitz {
        Blitz {
            app: Rc::default(),
            owner: dioxus::core::current_scope_id(),
        }
    }

    /// Run `f` as the shell, so what it spawns lives as long as the window.
    fn as_shell(&self, f: impl FnOnce()) {
        dioxus::core::Runtime::current().in_scope(self.owner, f);
    }

    /// `.app` has mounted: keep its handle and give it the keyboard.
    pub(in crate::ui) fn mounted(&self, app: Rc<MountedData>) {
        self.app.replace(Some(Rc::clone(&app)));
        self.as_shell(|| ds::focus_soon(app));
    }

    /// The window's state changed (a menu, a panel or a sheet opened or closed): a frame later,
    /// if that left the keyboard nowhere, `.app` takes it back.
    ///
    /// `FocusFallback::Ancestor` covers a click on nothing focusable, not an element that had the
    /// keyboard going away under it: a menu quire closes on Escape, a sheet, the palette. Blitz
    /// then leaves the focus nowhere, and every key would go to the document's root. quire's own
    /// click-focus `restore` does the rest: it focuses `.app` only if the focus is nowhere, so a
    /// field, a menu or the palette that has it keeps it.
    pub(in crate::ui) fn hand_back(&self) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        self.as_shell(|| {
            spawn(async move {
                ds::sleep(ds::FRAME_SLACK).await;
                let _ = (ds_native::CLICK_FOCUS.restore)(&app);
            });
        });
    }

    /// Answer `ask`, from a handler; the work is a task of the shell's.
    pub(in crate::ui) fn ask(&self, ask: Ask) {
        match ask {
            Ask::FocusApp => {
                if let Some(app) = self.app.borrow().clone() {
                    self.as_shell(|| ds::focus_soon(app));
                }
            }
            Ask::Focus { selector, .. } => self.focus(selector, Select::None),
            Ask::FocusAndSelect(field) => self.focus(field.selector(), Select::All),
            Ask::ScrollIntoView(selector) => self.scroll(selector),
            Ask::Copy(text) => {
                // Best-effort, as the webview's `navigator.clipboard` was: a desktop with no
                // clipboard leaves the address where it was, and nothing else depends on it.
                let _ = ds_native::clipboard::write_text(&text);
            }
        }
    }

    /// Focus the first element `selector` matches once it is drawn, and do `select` with its
    /// text. Best-effort, as the webview's scripts were: an element that never shows is no error.
    fn focus(&self, selector: &'static str, select: Select) {
        self.as_shell(|| {
            spawn(async move {
                let _ = ds::focus_by_selector(selector, select).await;
            });
        });
    }

    /// Bring the first element `selector` matches into the middle of its scroller, now or on a
    /// later frame.
    fn scroll(&self, selector: &'static str) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        self.as_shell(|| {
            spawn(async move {
                // A scroll is asked for the render that moved the hit, which has not been laid
                // out yet: a frame first, as the webview's two `requestAnimationFrame`s waited.
                ds::sleep(ds::FRAME_SLACK).await;
                for _ in 0..TRIES {
                    match scroll_into_view(&app, selector) {
                        Tried::Done | Tried::Never => return,
                        Tried::Later => ds::sleep(ds::FRAME_SLACK).await,
                    }
                }
            });
        });
    }
}

/// One try at scrolling the first element `selector` matches in `app`'s document into view.
fn scroll_into_view(app: &MountedData, selector: &str) -> Tried {
    let Some(handle) = app.downcast::<NodeHandle>() else {
        return Tried::Never;
    };
    // Found under a shared borrow, and only then written under an exclusive one: nothing runs
    // between the two on this thread, so the write cannot meet a borrow the read did not see.
    let found = match handle.try_doc() {
        None => return Tried::Later,
        Some(doc) => match doc.query_selector(selector) {
            Err(_) => return Tried::Never,
            Ok(None) => return Tried::Later,
            Ok(Some(node)) => node,
        },
    };
    let mut doc = handle.doc_mut();
    doc.scroll_into_view(
        found,
        ScrollBehavior::Instant,
        ScrollLogicalPosition::Center,
        ScrollLogicalPosition::Nearest,
    );
    doc.shell_provider.request_redraw();
    Tried::Done
}
