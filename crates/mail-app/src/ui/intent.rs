//! What a heard key means to the window it landed in, and the part of it both windows do alike.
//!
//! The main window and a conversation's own window each used to map `Heard` to an action in
//! their own `onkeydown`, and the two drifted (the thread window's Pin skipped the undo the main
//! window's row actions give). The mapping is one pure function here, [`intent`], and what a key
//! does to one conversation is [`on_thread`], which both windows call.

use super::actions::{Heard, Own};
use super::motion::act_kind;
use super::ops::{Composes, Writes, open_composer};
use super::reading::Raw;
use super::view::{Shell, Shortcut};
use crate::settings::ZoomStep;
use chordkit::StandardAction;
use dioxus::prelude::*;
use mail_core::SqliteStore;
use mail_core::search::Step;
use mail_domain::*;

/// Where the key landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Scope {
    /// The main window, with a list to step through and rows to act on.
    List,
    /// A conversation's own window: one conversation, no list, no search or Spaces.
    Window,
    /// A text field or the composer has the keyboard: only what a field cannot own.
    Typing,
}

/// What the window does for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Intent {
    ToggleSidebar,
    Print,
    /// ⌘K: bring up the search panel.
    Search,
    /// ⌘F: find in the open conversation.
    Find,
    /// ⌘G and ⇧⌘G (F3 and Shift+F3 in CUA): the next or previous match of that find.
    FindStep(Step),
    Settings,
    SelectAll,
    OpenInWindow,
    /// ⌘+, ⌘− and ⌘0: the reader's text a size bigger, smaller, or its own.
    Zoom(ZoomStep),
    /// ⇧⌘H and ⌥⌘R: the open conversation's newest message as it came.
    Raw(Raw),
    /// A mail key, Esc, ⌘N (as [`Shortcut::Compose`]) or Move to Trash (as [`Shortcut::Trash`]).
    Mail(Shortcut),
}

/// What `heard` asks of a window of this `scope`, if anything.
///
/// A field keeps ⌘A and Shift+Enter and Move to Trash for itself; a thread window has no sidebar, search,
/// settings, list to step through or other conversation to start, so those are nothing there.
/// Finding in the conversation is every window's, and a field's too: the find bar's own field
/// hands ⌘F and ⌘G on.
pub(in crate::ui) fn intent(heard: Option<Heard>, scope: Scope) -> Option<Intent> {
    let chosen = match heard? {
        Heard::Mail(action) => Intent::Mail(action),
        Heard::Standard(StandardAction::Print) => Intent::Print,
        Heard::Standard(StandardAction::ToggleSidebar) => Intent::ToggleSidebar,
        Heard::Own(Own::Search) => Intent::Search,
        Heard::Standard(StandardAction::Find) => Intent::Find,
        Heard::Standard(StandardAction::FindNext) => Intent::FindStep(Step::Next),
        Heard::Standard(StandardAction::FindPrevious) => Intent::FindStep(Step::Previous),
        Heard::Standard(StandardAction::Settings) => Intent::Settings,
        Heard::Standard(StandardAction::SelectAll) => Intent::SelectAll,
        Heard::Standard(StandardAction::New) => Intent::Mail(Shortcut::Compose),
        Heard::Own(Own::OpenInWindow) => Intent::OpenInWindow,
        Heard::Standard(StandardAction::Bigger) => Intent::Zoom(ZoomStep::Bigger),
        Heard::Standard(StandardAction::Smaller) => Intent::Zoom(ZoomStep::Smaller),
        Heard::Standard(StandardAction::ActualSize) => Intent::Zoom(ZoomStep::ActualSize),
        Heard::Own(Own::ShowHeaders) => Intent::Raw(Raw::Headers),
        Heard::Own(Own::ViewSource) => Intent::Raw(Raw::Source),
        // Move to Trash (⌘⌫, or Delete in CUA) trashes what is open, as `#` does; a field keeps
        // the press for itself.
        Heard::Standard(StandardAction::MoveToTrash) if scope != Scope::Typing => {
            Intent::Mail(Shortcut::Trash)
        }
        _ => return None,
    };
    let allowed = match (scope, chosen) {
        (_, Intent::Print | Intent::Zoom(_) | Intent::Find | Intent::FindStep(_)) => true,
        (Scope::Window, Intent::Raw(_)) => true,
        (Scope::Typing, Intent::Raw(_)) => false,
        (Scope::Window, Intent::Mail(action)) => !matches!(
            action,
            Shortcut::Next
                | Shortcut::Previous
                | Shortcut::ExtendNext
                | Shortcut::ExtendPrevious
                | Shortcut::Compose
        ),
        (Scope::Window, _) => false,
        (Scope::Typing, Intent::SelectAll | Intent::OpenInWindow) => false,
        (Scope::List | Scope::Typing, _) => true,
    };
    allowed.then_some(chosen)
}

/// The keys that mean the same on one open conversation in either window: Pin goes through
/// `act_kind` like the row's own button, so it has its undo and its toast, and a reply or
/// forward opens the composer on it.
pub(in crate::ui) fn on_thread(
    store: &SqliteStore,
    shell: Signal<Shell>,
    mut revision: Signal<u64>,
    thread: ThreadId,
    action: Shortcut,
) {
    let what = match action {
        Shortcut::TogglePin => {
            act_kind(store, shell, revision, thread, OpKind::Pin);
            return;
        }
        Shortcut::Reply => Composes::Reply(ReplyScope::Sender),
        Shortcut::ReplyAll => Composes::Reply(ReplyScope::All),
        Shortcut::Forward => Composes::Forward,
        _ => return,
    };
    let writes = Writes::answering(&shell.peek(), thread, what);
    if open_composer(store, shell, writes).is_ok() {
        revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(shortcut: Shortcut) -> Option<Heard> {
        Some(Heard::Mail(shortcut))
    }

    #[test]
    fn a_heard_key_means_what_its_scope_lets_it() {
        use Intent::*;
        use Scope::*;
        let standard = |action| Some(Heard::Standard(action));
        let own = |own| Some(Heard::Own(own));
        let table = [
            (None, List, None),
            (mail(Shortcut::Archive), List, Some(Mail(Shortcut::Archive))),
            (
                mail(Shortcut::Archive),
                Window,
                Some(Mail(Shortcut::Archive)),
            ),
            (
                mail(Shortcut::TogglePin),
                Window,
                Some(Mail(Shortcut::TogglePin)),
            ),
            (mail(Shortcut::Back), Typing, Some(Mail(Shortcut::Back))),
            (mail(Shortcut::Next), List, Some(Mail(Shortcut::Next))),
            (mail(Shortcut::Next), Window, None),
            (mail(Shortcut::ExtendPrevious), Window, None),
            (mail(Shortcut::Compose), Window, None),
            (
                standard(StandardAction::New),
                List,
                Some(Mail(Shortcut::Compose)),
            ),
            (
                standard(StandardAction::New),
                Typing,
                Some(Mail(Shortcut::Compose)),
            ),
            (standard(StandardAction::New), Window, None),
            (standard(StandardAction::Print), List, Some(Print)),
            (standard(StandardAction::Print), Window, Some(Print)),
            (standard(StandardAction::Print), Typing, Some(Print)),
            (
                standard(StandardAction::ToggleSidebar),
                List,
                Some(ToggleSidebar),
            ),
            (standard(StandardAction::ToggleSidebar), Window, None),
            (standard(StandardAction::Find), List, Some(Find)),
            (standard(StandardAction::Find), Window, Some(Find)),
            (standard(StandardAction::Find), Typing, Some(Find)),
            (
                standard(StandardAction::FindNext),
                Window,
                Some(FindStep(Step::Next)),
            ),
            (
                standard(StandardAction::FindPrevious),
                Typing,
                Some(FindStep(Step::Previous)),
            ),
            (own(Own::Search), List, Some(Search)),
            (own(Own::Search), Typing, Some(Search)),
            (own(Own::Search), Window, None),
            (standard(StandardAction::Settings), List, Some(Settings)),
            (standard(StandardAction::SelectAll), List, Some(SelectAll)),
            (standard(StandardAction::SelectAll), Typing, None),
            (standard(StandardAction::SelectAll), Window, None),
            (own(Own::OpenInWindow), List, Some(OpenInWindow)),
            (own(Own::OpenInWindow), Typing, None),
            (own(Own::OpenInWindow), Window, None),
            (
                standard(StandardAction::MoveToTrash),
                List,
                Some(Mail(Shortcut::Trash)),
            ),
            (
                standard(StandardAction::MoveToTrash),
                Window,
                Some(Mail(Shortcut::Trash)),
            ),
            (standard(StandardAction::MoveToTrash), Typing, None),
            (own(Own::Send), List, None),
            (standard(StandardAction::Undo), List, None),
            (
                standard(StandardAction::Bigger),
                List,
                Some(Zoom(ZoomStep::Bigger)),
            ),
            (
                standard(StandardAction::Smaller),
                Window,
                Some(Zoom(ZoomStep::Smaller)),
            ),
            (
                standard(StandardAction::ActualSize),
                Typing,
                Some(Zoom(ZoomStep::ActualSize)),
            ),
            (
                own(Own::ShowHeaders),
                List,
                Some(Intent::Raw(crate::ui::reading::Raw::Headers)),
            ),
            (
                own(Own::ViewSource),
                Window,
                Some(Intent::Raw(crate::ui::reading::Raw::Source)),
            ),
            (own(Own::ViewSource), Typing, None),
        ];
        for (heard, scope, want) in table {
            assert_eq!(intent(heard, scope), want, "{heard:?} in {scope:?}");
        }
    }
}
