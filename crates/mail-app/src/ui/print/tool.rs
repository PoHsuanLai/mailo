//! The reader head's Print tool, and the small menu it opens.

use super::super::menu::anchor_at;
use super::super::press::on_primary;
use super::{Job, print, save};
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::controls::button_model::{Answers, Bezel, ImagePosition};
use ds::components::controls::segmented::Tracking;
use ds::host::measure::{Anchor, MountedRef};
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use mail_domain::ThreadId;
use mail_mime::Pages;

/// The two ways a conversation meets the page, as the menu names them.
pub(super) const CHOICES: [(Pages, &str); 2] = [
    (Pages::Flow, "Whole conversation"),
    (Pages::PerMessage, "Each message on its own page"),
];

/// Print, in the head's tools. It opens a popover rather than printing at once: the choice of
/// pages is there, and Save for printing beside it. Ctrl P prints without asking.
#[component]
pub(in crate::ui) fn PrintTool(thread: ThreadId) -> Element {
    let mut open = use_signal(|| Shown::Hidden);
    let mut tool = use_signal(|| None::<MountedRef>);
    let pages = use_signal(|| Pages::Flow);
    let label = "Print this conversation";
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            label,
            icon: Some(IconSource::Glyph(Icon::Printer)),
            shown: Some(open()),
            onclick: move |_| open.set(if open() == Shown::Visible { Shown::Hidden } else { Shown::Visible }),
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| tool.set(Some(MountedRef(event.data()))))),
                ..Common::default()
            },
        }
        if open() == Shown::Visible {
            PrintMenu { thread, pages, anchor: anchor_at(tool()), onclose: move |()| open.set(Shown::Hidden) }
        }
    }
}

/// The choice of pages, Print, and Save for printing.
#[component]
fn PrintMenu(
    thread: ThreadId,
    pages: Signal<Pages>,
    anchor: Anchor,
    onclose: EventHandler<()>,
) -> Element {
    let mut pages = pages;
    let job = Job {
        thread,
        pages: pages(),
    };
    let choices: Vec<Choice<Pages>> = CHOICES
        .into_iter()
        .map(|(choice, name)| Choice::new(choice, name))
        .collect();
    rsx! {
            Popover {
                anchor,
                placement: Placement::new(Side::Bottom, Align::End),
                gap: Px(4.0),
                onclose,
                common: Common { extra_class: ExtraClass::parse("print-menu").ok(), ..Common::default() },
                SegmentedControl::<Pages> {
                    label: "Pages",
                    choices,
                    tracking: Tracking::SelectOne(pages()),
                    onchange: move |choice| pages.set(choice),
                }
                div { class: "acts",
                    Button {
                        label: "Save for printing…",
                        onclick: on_primary(move || {
                            onclose.call(());
                            save(job);
                        }),
        common: Common { aria_label: Some("Save for printing…".to_owned()), ..Common::default() },
    }
                    Button {
                        label: "Print",
                        answers: Answers::Return,
                        onclick: on_primary(move || {
                            onclose.call(());
                            print(job);
                        }),
        common: Common { aria_label: Some("Print".to_owned()), ..Common::default() },
    }
                }
            }
        }
}
