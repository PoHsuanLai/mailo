//! The words the window's tests drive quire's `Harness` with.
//!
//! `ds_harness` has one way in, `Driver::send(Input::..)`, and reads through `Query`. The tests
//! here click, press and chord hundreds of times, so this names each of those once and leaves
//! every call a line that reads as what a person did. Nothing here reaches past the harness's
//! own inputs: each method is one `Input` sent, and the reads are `Query`'s.
//!
//! Included by the integration tests as `mod support` and by the unit tests through `#[path]`,
//! so there is one copy.

#![allow(dead_code)]

use dioxus::html::Modifiers;
use ds::base::press::PointerButton;
use ds::file_drop::drag::{DropAcceptance, FileDragInput};
use ds::prelude::*;
use ds_harness::{Driver, FocusState, Harness, Input, PointerAction, PointerInput, Query};
use std::time::Duration;

/// A key the tests press: the words the window's own shortcuts are written in.
pub use ds::prelude::ShortcutKey as Key;

/// Sending input the way a person gives it.
pub trait Drive {
    /// Click the primary button at `at`.
    fn click(&mut self, at: Point);
    /// Click the primary button at `at` with `held` modifiers down.
    fn click_with(&mut self, at: Point, held: &[Key]);
    /// Move the pointer to `at`.
    fn pointer_move(&mut self, at: Point);
    /// Press and release `key`.
    fn key(&mut self, key: Key);
    /// Press and release `key` with `held` modifiers down.
    fn chord(&mut self, held: &[Key], key: Key);
    /// Scroll by `dx`, `dy` with the pointer at `at`.
    fn wheel(&mut self, at: Point, dx: Px, dy: Px);
    /// Click `button` at `at`.
    fn press(&mut self, at: Point, button: PointerButton);
    /// One step of a file drag from outside the window. What the window told the platform.
    fn file_drag(&mut self, input: FileDragInput) -> DropAcceptance;
    /// Drag the primary button from `from` to `to` in `steps` moves.
    fn drag(&mut self, from: Point, to: Point, steps: u16);
    /// Paste `html` with its plain `text`.
    fn paste_html(&mut self, html: &str, text: &str);
    /// The IME attaches.
    fn ime_start(&mut self);
    /// The IME shows `text` as its preedit.
    fn ime_update(&mut self, text: &str, cursor: usize);
    /// The IME commits `text`.
    fn ime_commit(&mut self, text: &str);
    /// The IME detaches.
    fn ime_end(&mut self);
    /// Let `ms` milliseconds pass.
    fn wait(&mut self, ms: u64);
    /// Whether `selector` has the keyboard.
    fn is_focused(&self, selector: &str) -> bool;
}

impl Drive for Harness {
    fn click(&mut self, at: Point) {
        self.send(Input::click(at));
    }
    fn click_with(&mut self, at: Point, held: &[Key]) {
        let mods = held.iter().fold(Modifiers::empty(), |all, key| {
            all | match key {
                Key::Ctrl => Modifiers::CONTROL,
                Key::Shift => Modifiers::SHIFT,
                Key::Alt => Modifiers::ALT,
                Key::Super => Modifiers::META,
                _ => Modifiers::empty(),
            }
        });
        self.send(Input::Pointer(
            PointerInput::new(at, PointerAction::Click(PointerButton::Primary)).with_mods(mods),
        ));
    }
    fn pointer_move(&mut self, at: Point) {
        self.send(Input::pointer_move(at));
    }
    fn key(&mut self, key: Key) {
        self.send(Input::key(key));
    }
    fn chord(&mut self, held: &[Key], key: Key) {
        self.send(Input::chord(held, key));
    }
    fn wheel(&mut self, at: Point, dx: Px, dy: Px) {
        self.send(Input::wheel(at, dx, dy));
    }
    fn press(&mut self, at: Point, button: PointerButton) {
        self.send(Input::press(at, button));
    }
    fn file_drag(&mut self, input: FileDragInput) -> DropAcceptance {
        self.send(Input::FileDrag(input));
        self.drop_answer()
    }
    fn drag(&mut self, from: Point, to: Point, steps: u16) {
        self.send(Input::drag(from, to, steps));
    }
    fn paste_html(&mut self, html: &str, text: &str) {
        self.send(Input::paste(html, text));
    }
    fn ime_start(&mut self) {
        self.send(Input::ime_start());
    }
    fn ime_update(&mut self, text: &str, cursor: usize) {
        self.send(Input::ime_update(text, cursor));
    }
    fn ime_commit(&mut self, text: &str) {
        self.send(Input::ime_commit(text));
    }
    fn ime_end(&mut self) {
        self.send(Input::ime_end());
    }
    fn wait(&mut self, ms: u64) {
        self.advance(Duration::from_millis(ms));
    }
    fn is_focused(&self, selector: &str) -> bool {
        self.focus_of(selector) == FocusState::Focused
    }
}
