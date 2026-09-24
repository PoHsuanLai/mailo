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

/// A pointer at a place on the page: `client` in the window, `offset` inside its target.
#[derive(Debug, Clone)]
pub(in crate::ui) struct FakePointer {
    pub(in crate::ui) client: (f64, f64),
    pub(in crate::ui) offset: (f64, f64),
    pub(in crate::ui) held: bool,
}

impl dioxus::html::point_interaction::ModifiersInteraction for FakePointer {
    fn modifiers(&self) -> Modifiers {
        Modifiers::empty()
    }
}

impl dioxus::html::point_interaction::InteractionLocation for FakePointer {
    fn client_coordinates(&self) -> dioxus::html::geometry::ClientPoint {
        dioxus::html::geometry::ClientPoint::new(self.client.0, self.client.1)
    }
    fn screen_coordinates(&self) -> dioxus::html::geometry::ScreenPoint {
        dioxus::html::geometry::ScreenPoint::new(self.client.0, self.client.1)
    }
    fn page_coordinates(&self) -> dioxus::html::geometry::PagePoint {
        dioxus::html::geometry::PagePoint::new(self.client.0, self.client.1)
    }
}

impl dioxus::html::point_interaction::InteractionElementOffset for FakePointer {
    fn element_coordinates(&self) -> dioxus::html::geometry::ElementPoint {
        dioxus::html::geometry::ElementPoint::new(self.offset.0, self.offset.1)
    }
}

impl dioxus::html::point_interaction::PointerInteraction for FakePointer {
    fn trigger_button(&self) -> Option<dioxus::html::input_data::MouseButton> {
        Some(dioxus::html::input_data::MouseButton::Primary)
    }
    fn held_buttons(&self) -> dioxus::html::input_data::MouseButtonSet {
        let mut set = dioxus::html::input_data::MouseButtonSet::empty();
        if self.held {
            set.insert(dioxus::html::input_data::MouseButton::Primary);
        }
        set
    }
}

impl dioxus::html::HasPointerData for FakePointer {
    fn pointer_id(&self) -> i32 {
        1
    }
    fn width(&self) -> f64 {
        1.0
    }
    fn height(&self) -> f64 {
        1.0
    }
    fn pressure(&self) -> f32 {
        0.0
    }
    fn tangential_pressure(&self) -> f32 {
        0.0
    }
    fn tilt_x(&self) -> i32 {
        0
    }
    fn tilt_y(&self) -> i32 {
        0
    }
    fn twist(&self) -> i32 {
        0
    }
    fn pointer_type(&self) -> String {
        "mouse".to_owned()
    }
    fn is_primary(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Send a pointer event named `name` (`"pointerover"`, `"pointerdown"`…) to `element`.
pub(in crate::ui) fn pointer(
    dom: &mut VirtualDom,
    name: &str,
    element: ElementId,
    at: FakePointer,
) -> Seen {
    dispatch(dom, name, PlatformEventData::new(Box::new(at)), element)
}
