//! Connections: a sheet that lists every account that has a server, how it stands and the one
//! thing to do about it, as macOS Mail's Connection Doctor does.
//!
//! It is the full view and nothing routes to it by itself: an account that needs signing in is
//! asked about in a dialog (`reauth`), and the sidebar, the list and Settings each say it in their
//! own words with a Sign In beside it. This sheet is reached from the command menu, from
//! Settings' account page, and from an account tile that has trouble a sign-in cannot fix. The
//! words of each line are [`crate::ui::fetching::account_line`]'s.
//!
//! Sign In is the dialog's (`Reauth::sign_in`): straight to the browser for an account that signs
//! in with a redirect, else the Add account window with the address typed in. Settings borrows
//! the same window. This sheet steps aside while it is open and comes back when it closes, and
//! what the account was waiting for is told to its link once the Add account window has changed
//! something. Each row's gear opens Settings on the account's own page
//! (`settings_window::open_account`).

use crate::ui::common::in_card;
use crate::ui::data::account_rows;
use crate::ui::fetching::{AccountLine, Fetching, Need, Remedy, Standing, account_line, offered};
use crate::ui::press::{SheetClose, on_primary};
use crate::ui::reauth::Reauth;
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
use mail_core::SqliteStore;
use mail_core::fetch::{Event, Link, Trigger};
use porter_core::AccountId;
use std::sync::Arc;

/// What the sheet and its command are called.
pub(in crate::ui) const TITLE: &str = "Connections";

/// Open the sheet.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().doctor = Some(DoctorSheet);
}

/// Close the sheet.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().doctor = None;
    crate::ui::host::Host::focus_app();
}

/// The button's label.
fn label_of(remedy: Remedy) -> &'static str {
    match remedy {
        Remedy::SignIn => "Sign In",
        Remedy::Allow => "Allow Mail\u{2026}",
        Remedy::TryAgain => "Try Again",
        Remedy::Settings => "Settings\u{2026}",
        Remedy::UseGraph => "Use Microsoft Graph",
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
    let mut pending = use_signal(|| None::<(AccountId, u64)>);
    // The Add account window changed an account's settings: it moves the shared revision when it
    // does, and a window closed with nothing changed leaves the account as it was.
    use_effect(move || {
        let now = revision();
        let Some((account, at)) = pending.peek().clone() else {
            return;
        };
        if now == at {
            return;
        }
        pending.set(None);
        let Some(fetching) = fetching else { return };
        fetching.send(account, Event::Start(Trigger::Manual));
    });
    let Some(fetching) = fetching else {
        return rsx! {};
    };
    let accounts = listed(fetching, &consume_context::<Arc<SqliteStore>>());
    let now = crate::ui::clock::now();
    let busy = accounts.iter().any(|(_, _, link)| link.is_busy());
    let reauth = try_consume_context::<Reauth>();
    let act = Callback::new(
        move |(account, name, remedy): (AccountId, String, Remedy)| match remedy {
            Remedy::UseGraph => use_graph(fetching, revision, account, name),
            Remedy::TryAgain => fetching.send(account, Event::Start(Trigger::Manual)),
            Remedy::Allow => {
                if let Some(reauth) = reauth {
                    reauth.sign_in(&name, Need::Allow);
                }
            }
            // Straight to the browser when the account signs in with a redirect, as it was
            // added; the window, with the address typed, otherwise.
            Remedy::SignIn => {
                if let Some(reauth) = reauth {
                    reauth.sign_in(&name, Need::SignIn);
                }
            }
            Remedy::Settings => {
                pending.set(Some((account, *revision.peek())));
                crate::ui::add_account::open_for(name);
            }
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
                        line: offered(
                            account_line(&link, fetching.pace(&account), now, &chrono::Local),
                            &link,
                            fetching.offer(&account),
                        ),
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

/// Use Microsoft Graph: set the account up again over Graph and sign it in for Graph in the
/// browser (`mail_core::account::switch_to_graph`), then fetch with the new sign-in.
///
/// The sign-in waits on a browser, so it runs on a thread of its own, as the Add account sheet's
/// does; what it came to is said, and a sign-in that worked is told to the account's link.
fn use_graph(fetching: Fetching, mut revision: Signal<u64>, account: AccountId, name: String) {
    let Some(store) = try_consume_context::<Arc<SqliteStore>>() else {
        return;
    };
    dioxus::core::spawn_forever(async move {
        let (done, answer) = tokio::sync::oneshot::channel();
        let switching = account.clone();
        std::thread::spawn(move || {
            let environment = crate::edge::environment();
            let secrets = crate::edge::secrets();
            let saved = mail_core::account::saved_clients();
            let browse = |url: &str| {
                if let Err(why) = webbrowser::open(url) {
                    log::warn!("the browser would not open the Microsoft sign-in: {why}");
                }
            };
            let switched = crate::edge::block_on(mail_core::account::switch_to_graph(
                &store,
                switching,
                chrono::Utc::now(),
                mail_core::account::Credentials {
                    password: None,
                    saved: &saved,
                    secrets: secrets.as_ref(),
                    environment: &environment,
                    on_url: &browse,
                    signed: None,
                },
            ));
            // The window that asked may have closed; then nobody is left to tell.
            let _ = done.send(switched.map(|added| added.outcome).map_err(String::from));
        });
        let said = match answer.await {
            Ok(Ok(mail_core::account::Outcome::SignedIn { .. })) => {
                revision += 1;
                fetching.signed_in(account);
                format!("{name} now reads and sends through Microsoft Graph.")
            }
            Ok(Ok(_)) => {
                revision += 1;
                format!("{name} is set up for Microsoft Graph, and waits to be signed in.")
            }
            Ok(Err(why)) => format!("Couldn\u{2019}t move {name} to Microsoft Graph: {why}"),
            Err(_) => format!("Couldn\u{2019}t move {name} to Microsoft Graph: it stopped."),
        };
        crate::ui::motion::tell(said, crate::ui::motion::Follow::Nothing);
    });
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
