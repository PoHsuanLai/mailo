//! Print…, a further menu off the reader's ⋯: the Mac's File menu's printing commands, hung
//! under the ⋯ as the reader's other menus are.

use super::super::menu::anchor_at;
use super::{Job, print, save};
use dioxus::prelude::*;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use mail_domain::ThreadId;
use mail_reader::Pages;

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

/// Print…, from the reader's ⋯: one page flow or a page per message, and saving as a PDF, hung
/// where the ⋯ hangs. ⌘P prints the whole conversation without asking.
#[component]
pub(in crate::ui) fn PrintMenu(
    thread: ThreadId,
    anchor: Option<MountedRef>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        Menu::<PrintChoice> {
            placement: MenuPlacement::Popup,
            anchor: anchor_at(anchor),
            items: rows(),
            onpick: move |choice: PrintChoice| {
                on_close.call(());
                let job = job_of(thread, choice);
                match choice {
                    PrintChoice::Print(_) => print(job),
                    PrintChoice::Save => save(job),
                }
            },
            onclose: move |()| on_close.call(()),
        }
    }
}
