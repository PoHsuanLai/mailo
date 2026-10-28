//! Settings › General's calendar row: which calendar Add to Calendar puts events in.
//!
//! The calendars are listed only when the person presses Choose: listing them may make the system
//! ask for access to their calendars, and opening Settings is no reason to ask. Until then the
//! row says what the setting holds. The choice is `calendar.default`, an id the store gave the
//! calendar; an empty one is the first calendar that takes events.

use crate::calendar::Calendars;
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::menus::item::item::MenuItem;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use porter_calendar::CalendarRef;

/// The pop-up's item for "whichever takes events first": no calendar has an empty id.
const FIRST: &str = "";

/// The button that lists the calendars.
pub const CHOOSE_ID: &str = "calendar-choose";

/// The items of the pop-up: the first-available choice, then each calendar that takes events.
pub(super) fn items_of(calendars: &[CalendarRef]) -> Vec<(String, String)> {
    std::iter::once((FIRST.to_owned(), copy::fields::FIRST_CALENDAR.to_owned()))
        .chain(
            calendars
                .iter()
                .filter(|calendar| calendar.writable)
                .map(|calendar| (calendar.id.clone(), calendar.name.clone())),
        )
        .collect()
}

#[component]
pub(super) fn CalendarRow() -> Element {
    let Some(desk) = try_consume_context::<Calendars>() else {
        return rsx! {};
    };
    let mut listed = use_signal(|| None::<Vec<CalendarRef>>);
    let mut failed = use_signal(|| None::<String>);
    let chosen = crate::ui::prefs::current().calendar.default;
    let choose = on_primary(move || {
        let desk = desk.clone();
        spawn(async move {
            match mail_client::invite::calendars(&desk).await {
                found if found.is_empty() => {
                    failed.set(Some(copy::errors::NO_CALENDAR_TO_ADD_TO.to_owned()));
                }
                found => listed.set(Some(found)),
            }
        });
    });
    rsx! {
        FieldRow { label: copy::fields::CALENDAR_FOR_INVITATIONS,
            match listed() {
                None => rsx! {
                    Label {
                        text: if chosen.is_empty() { copy::fields::FIRST_CALENDAR } else { copy::fields::CHOSEN_CALENDAR },
                    }
                    Button {
                        label: copy::controls::CHOOSE_CALENDAR,
                        size: ControlSize::Small,
                        common: Common { id: Some(CHOOSE_ID.to_owned()), ..Common::default() },
                        onclick: choose,
                    }
                },
                Some(calendars) => rsx! {
                    PopUpButton::<String> {
                        items: items_of(&calendars)
                            .into_iter()
                            .map(|(id, name)| MenuItem::new(id, name))
                            .collect::<Vec<_>>(),
                        value: Some(chosen.clone()),
                        common: Common { aria_label: Some(copy::fields::CALENDAR_FOR_INVITATIONS.to_owned()), ..Common::default() },
                        onpick: move |id: String| {
                            let changed = crate::ui::prefs::change(move |settings| settings.calendar.default = id);
                            failed.set(changed.err());
                        },
                    }
                },
            }
        }
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| failed.set(None) }
        }
    }
}
