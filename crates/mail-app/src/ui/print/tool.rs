//! The reader head's Print tool, and the small menu it opens.

use super::{Job, print, save};
use dioxus::prelude::*;
use ds::{IconButton, IconButtonVariant, SegmentedControl, Switch};
use mail_domain::ThreadId;
use mail_mime::Pages;

/// The two ways a conversation meets the page, as the menu names them.
pub(super) const CHOICES: [(Pages, &str); 2] = [
    (Pages::Flow, "Whole conversation"),
    (Pages::PerMessage, "Each message on its own page"),
];

/// Print, in the head's tools. It opens a menu rather than printing at once: the choice of pages
/// is there, and Save for printing beside it. Ctrl P prints without asking.
#[component]
pub(in crate::ui) fn PrintTool(thread: ThreadId) -> Element {
    let mut open = use_signal(|| false);
    let pages = use_signal(|| Pages::Flow);
    let label = "Print this conversation";
    rsx! {
        IconButton {
            variant: IconButtonVariant::Tool,
            icon: ds::Icon::Printer,
            label: label.to_owned(),
            expanded: if open() { Switch::On } else { Switch::Off },
            onclick: move |_| open.toggle(),
        }
        if open() {
            PrintMenu { thread, pages, open }
        }
    }
}

/// The choice of pages, Print, and Save for printing.
#[component]
fn PrintMenu(thread: ThreadId, pages: Signal<Pages>, open: Signal<bool>) -> Element {
    let mut pages = pages;
    let mut open = open;
    let chosen = pages();
    let job = Job {
        thread,
        pages: chosen,
    };
    let print_label = "Print";
    let save_label = "Save for printing…";
    rsx! {
        div {
            class: "fmenu print-menu",
            role: "dialog",
            aria_label: "Print options",
            // Focusable, so the window's focus keeper lands here and Esc reaches it.
            tabindex: "-1",
            onkeydown: move |event| {
                if event.key().to_string() == "Escape" {
                    event.stop_propagation();
                    open.set(false);
                }
            },
            div { class: "g", "Print" }
            SegmentedControl::<Pages> {
                label: "Pages",
                options: CHOICES.into_iter().map(|(choice, name)| (choice, name.to_owned())).collect::<Vec<_>>(),
                value: chosen,
                onchange: move |choice| pages.set(choice),
            }
            div { class: "acts",
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "{save_label}",
                    onclick: move |_| {
                        open.set(false);
                        save(job);
                    },
                    "{save_label}"
                }
                button {
                    class: "mini primary",
                    r#type: "button",
                    aria_label: "{print_label}",
                    onclick: move |_| {
                        open.set(false);
                        print(job);
                    },
                    "{print_label}"
                }
            }
        }
    }
}
