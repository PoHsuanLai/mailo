//! The banner above the list, when an account needs something from the person.
//!
//! The words come from [`fetching::banner`]; this draws them as quire's `InlineBanner`, kept
//! mounted and driven by `shown` so that it slides in and out instead of jumping, and gives its
//! button something to do.

use crate::ui::common::{classed, in_card};
use crate::ui::data::account_rows;
use crate::ui::fetching::{AccountName, Banner, BannerAction, Fetching, Sev, banner};
use crate::ui::press::{SheetClose, on_primary};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::lists::row::row::Outline;
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::fetch::{Event, Link, Trigger};
use mail_domain::AccountId;
use mail_store::SqliteStore;
use std::sync::Arc;

/// What closing the add-account sheet should do for an account, if the sheet changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// The password was replaced: tell the link, which then tries again.
    SignedIn,
    /// The settings were changed: try again now.
    Retry,
}

/// The button's label.
fn label_of(action: &BannerAction) -> &'static str {
    match action {
        BannerAction::TryNow(_) => "Try Now",
        BannerAction::SignIn(_) => "Sign In",
        BannerAction::Settings(_) => "Account Settings",
        BannerAction::ShowAll => "Review",
    }
}

fn severity_of(sev: Sev) -> Severity {
    match sev {
        Sev::Info => Severity::Info,
        Sev::Warn => Severity::Warn,
        Sev::Danger => Severity::Danger,
    }
}

/// The accounts in view with their names and links, for [`banner`].
fn named(fetching: Fetching, shell: &Shell, store: &SqliteStore) -> Vec<(AccountId, String, Link)> {
    let rows = account_rows(store);
    fetching
        .links_in_view(shell)
        .into_iter()
        .filter_map(|(id, link)| {
            let row = rows.iter().find(|row| row.id == id)?;
            Some((id, row.shown(), link))
        })
        .collect()
}

/// The banner, and the sheet its "Review" opens.
#[component]
pub(super) fn FetchBanner(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let fetching = try_consume_context::<Fetching>();
    let mut reviewing = use_signal(|| false);
    let mut pending = use_signal(|| None::<(AccountId, u64, After)>);
    // The last banner said, so that one on its way out still has its words while it closes.
    let mut kept = use_hook(|| CopyValue::new(None::<Banner>));
    // The sheet that signed an account in, or changed its settings, has closed.
    use_effect(move || {
        if shell.read().adding.is_some() {
            return;
        }
        let Some((account, at, after)) = *pending.peek() else {
            return;
        };
        pending.set(None);
        let Some(fetching) = fetching else { return };
        if *revision.peek() == at {
            return;
        }
        match after {
            After::SignedIn => fetching.signed_in(account),
            After::Retry => fetching.send(account, Event::Start(Trigger::Manual)),
        }
    });
    let Some(fetching) = fetching else {
        return rsx! {};
    };
    let accounts = named(
        fetching,
        &shell.read(),
        &consume_context::<Arc<SqliteStore>>(),
    );
    let pairs: Vec<(AccountName, &Link)> = accounts
        .iter()
        .map(|(_, name, link)| (AccountName(name.clone()), link))
        .collect();
    let now = crate::ui::clock::now();
    let current = banner(&pairs, now, &chrono::Local);
    if let Some(now) = &current {
        kept.set(Some(now.clone()));
    }
    let ids: Vec<(AccountId, String)> = accounts
        .iter()
        .map(|(id, name, _)| (*id, name.clone()))
        .collect();
    let act = Callback::new(move |action: BannerAction| {
        let id_of = |name: &AccountName| ids.iter().find(|(_, one)| *one == name.0).map(|a| a.0);
        let name = match &action {
            BannerAction::TryNow(name)
            | BannerAction::SignIn(name)
            | BannerAction::Settings(name) => name,
            BannerAction::ShowAll => {
                reviewing.set(true);
                return;
            }
        };
        let Some(account) = id_of(name) else { return };
        match &action {
            BannerAction::TryNow(_) => fetching.send(account, Event::Start(Trigger::Manual)),
            BannerAction::SignIn(_) | BannerAction::Settings(_) => {
                let after = if matches!(action, BannerAction::SignIn(_)) {
                    After::SignedIn
                } else {
                    After::Retry
                };
                pending.set(Some((account, *revision.peek(), after)));
                shell.write().adding = Some(name.0.clone());
                crate::ui::host::Host::focus_next_frame(".acct-sheet input");
            }
            BannerAction::ShowAll => {}
        }
    });
    let shown = if current.is_some() {
        Shown::Visible
    } else {
        Shown::Hidden
    };
    let said = current.or_else(|| kept.read().clone());
    let sheet = reviewing().then(|| {
        let troubled: Vec<(AccountName, Banner)> = pairs
            .iter()
            .filter_map(|(name, link)| {
                banner(&[(name.clone(), *link)], now, &chrono::Local).map(|one| (name.clone(), one))
            })
            .collect();
        rsx! {
            Sheet {
                label: "Accounts that need attention",
                attach: Attach::Window,
                common: in_card(),
                onclose: move |()| reviewing.set(false),
                div { class: "attention",
                    Label { text: "Accounts that need attention", style: LabelStyle::Title }
                    for (name, one) in troubled {
                        Row {
                            key: "{name.0}",
                            title: name.0.clone(),
                            detail: Some(one.text.clone().into()),
                            outline: Outline::None,
                            accessory: Accessory::Slot(rsx! {
                                Button {
                                    size: ControlSize::Small,
                                    label: label_of(&one.action),
                                    onclick: {
                                        let action = one.action.clone();
                                        on_primary(move || {
                                            reviewing.set(false);
                                            act.call(action.clone());
                                        })
                                    },
                                }
                            }),
                        }
                    }
                    div { class: "attention-foot",
                        SheetClose { on_close: move |()| reviewing.set(false) }
                    }
                }
            }
        }
    });
    // Always mounted, `Hidden` until there is something to say, so that the first banner
    // slides in like every later one.
    let severity = said
        .as_ref()
        .map_or(Severity::Info, |said| severity_of(said.severity));
    let text = said
        .as_ref()
        .map(|said| said.text.clone())
        .unwrap_or_default();
    let button = said.map(|said| {
        let action = said.action.clone();
        rsx! {
            Button {
                size: ControlSize::Small,
                label: label_of(&said.action),
                onclick: on_primary(move || act.call(action.clone())),
            }
        }
    });
    rsx! {
        InlineBanner {
            severity,
            text,
            shown,
            actions: button,
            common: classed("fetch-banner"),
        }
        {sheet}
    }
}
