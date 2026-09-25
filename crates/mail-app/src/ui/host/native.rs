//! The window's asks answered on Blitz (`native`), where there is no script to evaluate.
//!
//! - The window itself, `.app`: its mounted handle is kept as it mounts, focused then so the
//!   keyboard has somewhere to land from the first key, and given the keyboard back through
//!   `ds::focus_soon`, which waits out a busy document.
//! - A field or element named by selector: quire's `TextInput` hands out no mounted handle, and
//!   `ds-native` makes none for an element found by a query, so there is nothing to give
//!   `ds::focus_soon`. The element is found in the Blitz document reached through `.app`'s
//!   handle and focused (and selected, or scrolled into view) with the same document writes
//!   `ds_native::focus` makes, retried a frame later while the document is busy or the element
//!   is not drawn yet, as the webview's scripts retried for twenty frames.
//! - The clipboard: `ds_native::clipboard::write_text`.

use super::Ask;
use blitz_dom::{BaseDocument, NodeId, ScrollBehavior, ScrollLogicalPosition};
use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use std::cell::RefCell;
use std::rc::Rc;

/// Frames an ask waits for its element to be drawn, as the webview's `focusFind` did.
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

/// What is done to the element a selector found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Focus,
    FocusAndSelect,
    ScrollIntoView,
}

/// One try at an ask.
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

    /// A pointer was released over the window: once the click is done, give `.app` the keyboard
    /// back if the click left it nowhere in particular.
    ///
    /// Blitz clears the focus on a click that lands on nothing it can focus, and `.app`'s
    /// `tabindex` does not make it catch that click, so after a click on a row every key would
    /// go to the document's root and `.app`'s key handler would never hear it. This is the part
    /// of the webview's `KEEP_FOCUS` script Blitz needs: only when the focus is nowhere, never
    /// taken from a field, a menu or the palette.
    pub(in crate::ui) fn keep_focus(&self) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        self.as_shell(|| {
            spawn(async move {
                ds::sleep(ds::FRAME_SLACK).await;
                if focus_is_nowhere(&app) {
                    ds::focus_soon(app);
                }
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
            Ask::Focus { selector, .. } => self.find(selector, Act::Focus),
            Ask::FocusAndSelect(field) => self.find(field.selector(), Act::FocusAndSelect),
            Ask::ScrollIntoView(selector) => self.find(selector, Act::ScrollIntoView),
            Ask::Copy(text) => {
                // Best-effort, as the webview's `navigator.clipboard` was: a desktop with no
                // clipboard leaves the address where it was, and nothing else depends on it.
                let _ = ds_native::clipboard::write_text(&text);
            }
        }
    }

    /// Find `selector` in the document and do `act` to it, now or on a later frame.
    fn find(&self, selector: &'static str, act: Act) {
        let Some(app) = self.app.borrow().clone() else {
            return;
        };
        self.as_shell(|| {
            spawn(async move {
                // A scroll is asked for the render that moved the hit, which has not been laid
                // out yet: a frame first, as the webview's two `requestAnimationFrame`s waited.
                if act == Act::ScrollIntoView {
                    ds::sleep(ds::FRAME_SLACK).await;
                }
                for _ in 0..TRIES {
                    match attempt(&app, selector, act) {
                        Tried::Done | Tried::Never => return,
                        Tried::Later => ds::sleep(ds::FRAME_SLACK).await,
                    }
                }
            });
        });
    }
}

/// One try at `act` on the first element `selector` matches in `app`'s document.
fn attempt(app: &MountedData, selector: &str, act: Act) -> Tried {
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
            Ok(Some(node)) if act == Act::FocusAndSelect && !laid_out_field(&doc, node) => {
                return Tried::Later;
            }
            Ok(Some(node)) => node,
        },
    };
    let mut doc = handle.doc_mut();
    match act {
        Act::Focus => {
            doc.set_focus_to(found);
        }
        Act::FocusAndSelect => {
            doc.set_focus_to(found);
            doc.with_text_input(found, |mut driver| driver.select_all());
        }
        Act::ScrollIntoView => doc.scroll_into_view(
            found,
            ScrollBehavior::Instant,
            ScrollLogicalPosition::Center,
            ScrollLogicalPosition::Nearest,
        ),
    }
    doc.shell_provider.request_redraw();
    Tried::Done
}

/// Whether `app`'s document has no element focused: Blitz reports its root element then. A
/// busy document is not known to be loose, so it is left alone.
fn focus_is_nowhere(app: &MountedData) -> bool {
    let Some(handle) = app.downcast::<NodeHandle>() else {
        return false;
    };
    let Some(doc) = handle.try_doc() else {
        return false;
    };
    let root = doc.try_root_element().map(|root| root.id);
    match doc.get_focussed_node_id() {
        None => true,
        Some(focused) => Some(focused) == root,
    }
}

/// Whether `node` is a text field Blitz has laid out, so its text can be selected.
fn laid_out_field(doc: &BaseDocument, node: NodeId) -> bool {
    doc.get_node(node)
        .and_then(|node| node.element_data())
        .is_some_and(|element| element.text_input_data().is_some())
}
