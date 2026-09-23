//! Chords, typing, and a document that remembers what the window asked it to run.
//!
//! Dispatched through `Runtime::handle_event`, the call `VirtualDom::handle_event` forwards to,
//! so these helpers need no `allow` for the deprecated one.

use super::dom::{Seen, Typed};
use dioxus::document::{Document, Eval, NoOpDocument};
use dioxus::html::input_data::keyboard_types::{Code, Key, Location, Modifiers};
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use std::cell::RefCell;
use std::rc::Rc;

/// A key pressed with modifiers held: Ctrl 2, Shift and an arrow.
#[derive(Debug, Clone)]
pub(in crate::ui) struct FakeChord(pub(in crate::ui) &'static str, pub(in crate::ui) Modifiers);

impl dioxus::html::point_interaction::ModifiersInteraction for FakeChord {
    fn modifiers(&self) -> Modifiers {
        self.1
    }
}

impl dioxus::html::HasKeyboardData for FakeChord {
    fn key(&self) -> Key {
        self.0.parse().unwrap_or(Key::Character(self.0.to_owned()))
    }
    fn code(&self) -> Code {
        Code::Unidentified
    }
    fn location(&self) -> Location {
        Location::Standard
    }
    fn is_auto_repeating(&self) -> bool {
        false
    }
    fn is_composing(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn dispatch(dom: &mut VirtualDom, name: &str, data: PlatformEventData, element: ElementId) -> Seen {
    let data: Rc<dyn std::any::Any> = Rc::new(data);
    dom.runtime()
        .handle_event(name, dioxus_core::Event::new(data, true), element);
    let mut seen = Seen::default();
    dom.render_immediate(&mut seen);
    seen
}

/// Press `key` with `modifiers` on `element`, and return what the next render set.
pub(in crate::ui) fn chord(
    dom: &mut VirtualDom,
    key: &'static str,
    modifiers: Modifiers,
    element: ElementId,
) -> Seen {
    let data = PlatformEventData::new(Box::new(FakeChord(key, modifiers)));
    dispatch(dom, "keydown", data, element)
}

/// Type `text` into the field `element`, as one input event carrying its new value.
pub(in crate::ui) fn type_into(dom: &mut VirtualDom, element: ElementId, text: &str) -> Seen {
    let data = PlatformEventData::new(Box::new(Typed(text.to_owned())));
    dispatch(dom, "input", data, element)
}

/// A document that keeps every script the window evals, in order, and runs none of them.
#[derive(Clone, Default)]
pub(in crate::ui) struct Scripts(Rc<RefCell<Vec<String>>>);

impl Scripts {
    /// The document to give the window as its root context.
    pub(in crate::ui) fn document(&self) -> Rc<dyn Document> {
        Rc::new(self.clone())
    }

    /// Everything evaluated so far.
    pub(in crate::ui) fn all(&self) -> Vec<String> {
        self.0.borrow().clone()
    }
}

impl Document for Scripts {
    fn eval(&self, js: String) -> Eval {
        self.0.borrow_mut().push(js.clone());
        NoOpDocument.eval(js)
    }
}
