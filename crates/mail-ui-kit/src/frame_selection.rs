//! What is selected in the reader's messages, for a reply to quote only that.
//!
//! Each message's body is a frame with a document of its own (`blocks::Sandbox`), and Blitz keeps
//! a selection per document: a drag in a message selects in that message's frame, and nowhere the
//! window's own document can see. So each frame, as it mounts, is put down here against its
//! message, and Reply asks the frames of the conversation it answers whether any holds a
//! selection. One that does is the message answered, and its selection is the whole of the quote.
//!
//! A window's, not a process's: every window that reads provides its own ([`use_selections`]),
//! so a selection in one window is never quoted by a reply started in another. A frame is held
//! here, since nothing else keeps its handle once it has mounted, and let go when its message is
//! drawn no more.

use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use mail_domain::MessageId;
use std::cell::RefCell;
use std::rc::Rc;

/// A message, and the frame its body is drawn in.
type Held = (MessageId, Rc<MountedData>);

/// The frames a window's readers draw, each against its message.
#[derive(Debug, Clone, Default)]
pub struct Selections(Rc<RefCell<Vec<Held>>>);

impl Selections {
    /// `frame` is `message`'s body now, in place of any frame it had.
    pub fn hold(&self, message: MessageId, frame: &Rc<MountedData>) {
        let mut held = self.0.borrow_mut();
        held.retain(|(id, _)| *id != message);
        held.push((message, frame.clone()));
    }

    /// `message` is drawn no more.
    pub fn let_go(&self, message: MessageId) {
        self.0.borrow_mut().retain(|(id, _)| *id != message);
    }

    /// The text selected in one of `messages`' frames, with the message it is in: the newest
    /// such message, where more than one frame still holds a selection. `messages` is in the
    /// conversation's order, oldest first.
    pub fn selected(&self, messages: &[MessageId]) -> Option<(MessageId, String)> {
        let held = self.0.borrow();
        messages.iter().rev().find_map(|message| {
            let (_, frame) = held.iter().find(|(id, _)| id == message)?;
            selected_in(frame).map(|text| (*message, text))
        })
    }
}

/// The window's frames: provided by every window that reads, once, before its readers.
pub fn use_selections() {
    use_context_provider(Selections::default);
}

/// What `frame`'s document has selected, when it is more than space.
fn selected_in(frame: &MountedData) -> Option<String> {
    let handle = frame.downcast::<NodeHandle>()?;
    let doc = handle.try_doc()?;
    let sub = doc.subdoc(handle.node_id())?;
    let inner = sub.inner();
    inner
        .get_selected_text()
        .filter(|text| !text.trim().is_empty())
}

/// The selection a reply to `messages` quotes, from the window's frames: `None` in a window that
/// keeps none (a test's reader alone), and wherever nothing is selected.
pub fn selected(messages: &[MessageId]) -> Option<(MessageId, String)> {
    try_consume_context::<Selections>()?.selected(messages)
}
