//! The Connection Doctor: a sheet that lists every account that has a server, how it stands and
//! the one thing to do about it, as macOS Mail's window of that name does.
//!
//! Mail marks an account with a problem in the sidebar and puts no banner over the messages;
//! pressing the mark opens this. Here the marks are `sidebar::marks`, the status line under the
//! list's title opens it too when it is a warning, and so does the search bar. The words of
//! each line are [`crate::ui::fetching::account_line`]'s.
//!
//! Sign In and Settings borrow the Add account sheet, prefilled with the account. This sheet
//! steps aside while it is open and comes back when it closes, and what the account was waiting
//! for is told to its link once the Add account sheet has changed something. Each row's gear
//! opens Settings on the account's own page (`settings_window::open_account`).

use crate::ui::common::in_card;
use crate::ui::data::account_rows;
use crate::ui::fetching::{AccountLine, Fetching, Remedy, Standing, account_line};
use crate::ui::press::{SheetClose, on_primary};
use crate::ui::view::{DoctorSheet, Shell};
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::progress::model::{Progress, ProgressStyle};
use ds::components::controls::progress::view::ProgressIndicator;
use ds::components::lists::row::row::Outline;
use ds::components::overlays::sheet_attach::Attach;
use ds::motion::detail::operation::{Operation, PendingToken};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_core::fetch::{Event, Link, Trigger};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// What the sheet and its command are called.
pub(in crate::ui) const TITLE: &str = "Connection Doctor";

/// Open the sheet.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().doctor = Some(DoctorSheet);
}

/// Close the sheet.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().doctor = None;
    crate::ui::host::Host::focus_app();
}

/// What closing the Add account sheet should do for an account, if it changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// The password was replaced: tell the link, which then tries again.
    SignedIn,
    /// The settings were changed: try again now.
    Retry,
}

/// The button's label.
fn label_of(remedy: Remedy) -> &'static str {
    match remedy {
        Remedy::SignIn => "Sign In",
        Remedy::Allow => "Allow Mail\u{2026}",
        Remedy::TryAgain => "Try Again",
        Remedy::Settings => "Settings\u{2026}",
    }
}

fn glyph_of(standing: Standing) -> Icon {
    match standing {
        Standing::Fine => Icon::CircleCheck,
        Standing::Working => Icon::Refresh,
        Standing::Warn | Standing::Broken => Icon::TriangleAlert,
        Standing::Offline => Icon::WifiOff,
    }
}

/// An account with a server: its id, how the window names it, and its link.
type Listed = (AccountId, String, Link);

/// Every account that has a server, in the order the window lists accounts.
fn listed(fetching: Fetching, store: &SqliteStore) -> Vec<Listed> {
    let links = fetching.all_links();
    account_rows(store)
        .into_iter()
        .filter_map(|row| {
            let (_, link) = links.iter().find(|(id, _)| *id == row.id)?;
            Some((row.id.clone(), row.shown(), link.clone()))
        })
        .collect()
}

/// The sheet. Mounted while `shell.doctor` is `Some`.
#[component]
pub(in crate::ui) fn DoctorView(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let fetching = try_consume_context::<Fetching>();
    let mut pending = use_signal(|| None::<(AccountId, u64, After)>);
    // The Add account window signed an account in, or changed its settings: it moves the shared
    // revision when it does, and a window closed with nothing added leaves the sign-in waiting.
    use_effect(move || {
        let now = revision();
        let Some((account, at, after)) = pending.peek().clone() else {
            return;
        };
        if now == at {
            return;
        }
        pending.set(None);
        let Some(fetching) = fetching else { return };
        match after {
            After::SignedIn => fetching.signed_in(account),
            After::Retry => fetching.send(account, Event::Start(Trigger::Manual)),
        }
    });
    let Some(fetching) = fetching else {
        return rsx! {};
    };
    let accounts = listed(fetching, &consume_context::<Arc<SqliteStore>>());
    let now = crate::ui::clock::now();
    let busy = accounts.iter().any(|(_, _, link)| link.is_busy());
    let act = Callback::new(
        move |(account, name, remedy): (AccountId, String, Remedy)| {
            let after = match remedy {
                Remedy::TryAgain => {
                    fetching.send(account, Event::Start(Trigger::Manual));
                    return;
                }
                Remedy::Allow => {
                    pending.set(Some((account, *revision.peek(), After::SignedIn)));
                    crate::ui::add_account::allow_again();
                    return;
                }
                Remedy::SignIn => After::SignedIn,
                Remedy::Settings => After::Retry,
            };
            pending.set(Some((account, *revision.peek(), after)));
            crate::ui::add_account::open_for(name);
        },
    );
    rsx! {
        Sheet {
            label: TITLE,
            attach: Attach::Window,
            common: in_card(),
            onclose: move |()| close(shell),
            div { class: "doctor",
                Label { text: TITLE, style: LabelStyle::Title }
                if accounts.is_empty() {
                    Label { text: "No accounts to check." }
                }
                for (account, name, link) in accounts {
                    AccountRowView {
                        key: "{account}",
                        name: name.clone(),
                        line: account_line(&link, now, &chrono::Local),
                        act: {
                            let account = account.clone();
                            move |remedy| act.call((account.clone(), name.clone(), remedy))
                        },
                        open: {
                            let account = account.clone();
                            move |()| crate::ui::settings_window::open_account(account.clone())
                        },
                    }
                }
                div { class: "doctor-foot",
                    Button {
                        label: "Check All",
                        availability: if busy { Availability::Busy } else { Availability::Enabled },
                        onclick: on_primary(move || fetching.sync_all(Trigger::Manual)),
                    }
                    SheetClose { label: "Done", on_close: move |()| close(shell) }
                }
            }
        }
    }
}

/// One account: its address, a glyph and how it stands, the button that fixes it, and the one
/// that opens its own sheet.
#[component]
fn AccountRowView(
    name: String,
    line: AccountLine,
    act: EventHandler<Remedy>,
    open: EventHandler<()>,
) -> Element {
    let AccountLine {
        standing,
        text,
        remedy,
    } = line;
    // Each named for its account, so that several Try Agains are not one name to a screen reader.
    let accessory = Accessory::Slot(rsx! {
        div { class: "doctor-acts",
            if let Some(remedy) = remedy {
                Button {
                    size: ControlSize::Small,
                    label: label_of(remedy),
                    common: Common {
                        aria_label: Some(format!("{} for {name}", label_of(remedy))),
                        ..Common::default()
                    },
                    onclick: on_primary(move || act.call(remedy)),
                }
            }
            if remedy.is_none() && standing == Standing::Working {
                Checking {}
            }
            Button {
                size: ControlSize::Small,
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                icon: Icon::Settings,
                label: format!("Account settings for {name}"),
                title: Some("Account Settings".to_owned()),
                onclick: on_primary(move || open.call(())),
            }
        }
    });
    rsx! {
        Row {
            leading: RowLeading::Icon(glyph_of(standing)),
            title: name,
            detail: Some(text.into()),
            outline: Outline::None,
            accessory,
        }
    }
}

/// quire's small spinner, for an account being checked: one operation for as long as it shows.
#[component]
fn Checking() -> Element {
    let token = use_hook(PendingToken::start);
    rsx! {
        ProgressIndicator {
            style: ProgressStyle::Spinner,
            progress: Progress::Unknown(Operation::Running(token)),
            size: ControlSize::Small,
        }
    }
}
