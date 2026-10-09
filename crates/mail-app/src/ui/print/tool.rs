//! The reader head's Print tool, and the pop-up menu it opens: the Mac's File menu's printing
//! commands, hung under the button as the reader's other menus are.

use super::super::menu::anchor_at;
use super::{Job, print, save};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::ThreadId;
use mail_mime::Pages;

/// What a row of the menu does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum PrintChoice {
    /// Print, with these pages.
    Print(Pages),
    /// Save the whole conversation as a PDF in the downloads directory.
    Save,
}

/// The menu's rows, in order: each way to print, a rule, then saving. `None` is the rule.
pub(in crate::ui) const ROWS: [Option<(PrintChoice, &str)>; 4] = [
    Some((PrintChoice::Print(Pages::Flow), "Print Conversation…")),
    Some((
        PrintChoice::Print(Pages::PerMessage),
        "Print Each Message Separately…",
    )),
    None,
    Some((PrintChoice::Save, "Save Conversation as PDF…")),
];

/// The job a row starts for `thread`: a print with the row's pages, or the whole conversation
/// saved.
pub(in crate::ui) fn job_of(thread: ThreadId, choice: PrintChoice) -> Job {
    let pages = match choice {
        PrintChoice::Print(pages) => pages,
        PrintChoice::Save => Pages::Flow,
    };
    Job { thread, pages }
}

fn rows() -> Vec<MenuItem<PrintChoice>> {
    ROWS.into_iter()
        .map(|row| match row {
            Some((choice, title)) => MenuItem::new(choice, title),
            None => MenuItem::Separator,
        })
        .collect()
}

/// Print, in the head's tools. It opens a menu rather than printing at once: one page flow or a
/// page per message, and saving as a PDF. ⌘P prints the whole conversation without asking.
#[component]
pub(in crate::ui) fn PrintTool(thread: ThreadId) -> Element {
    let mut open = use_signal(|| Shown::Hidden);
    let mut tool = use_signal(|| None::<MountedRef>);
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            label: "Print this conversation",
            title: Some("Print".to_owned()),
            title_shortcut: crate::ui::keymap::chord(&[crate::ui::keymap::KeyCap::Super], 'p'),
            icon: Some(IconSource::Glyph(Icon::Printer)),
            shown: Some(open()),
            onclick: move |_| open.set(open().flipped()),
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| tool.set(Some(MountedRef(event.data()))))),
                ..Common::default()
            },
        }
        if open() == Shown::Visible {
            Menu::<PrintChoice> {
                placement: MenuPlacement::Popup,
                anchor: anchor_at(tool()),
                items: rows(),
                onpick: move |choice: PrintChoice| {
                    open.set(Shown::Hidden);
                    let job = job_of(thread, choice);
                    match choice {
                        PrintChoice::Print(_) => print(job),
                        PrintChoice::Save => save(job),
                    }
                },
                onclose: move |()| open.set(Shown::Hidden),
            }
        }
    }
}
