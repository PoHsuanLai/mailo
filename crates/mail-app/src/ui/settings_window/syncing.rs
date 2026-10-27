//! How an account is synced, on its page beside Download All Mail: how often it is checked, how
//! far back its mail is downloaded, and, for POP3, when the server's copy of a message goes.
//!
//! Each choice is kept at once in `offline.json` ([`mail_core::offline`]), where every pass and
//! the scheduler read it, and the shared revision moves, so the main window's scheduler takes the
//! new interval, and starts or ends a push connection, without being opened again. A choice that
//! cannot be written is said in a dialog, and the menu stays as it was.

use crate::ui::appearance::WindowDirs;
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::data::AccountRow;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::menus::item::item::MenuItem;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::offline::{self, Every, Kept, Remove, Window};
use mail_domain::Incoming;

/// What Check for New Mail calls `every`.
pub(super) fn every_label(every: Every) -> String {
    match every {
        Every::Automatic => copy::controls::AUTOMATICALLY.to_owned(),
        Every::Minutes(minutes) => copy::controls::every_minutes(minutes),
        Every::Manually => copy::controls::MANUALLY.to_owned(),
    }
}

/// What Download Mail From calls `window`.
pub(super) fn window_label(window: Window) -> String {
    match window {
        Window::All => copy::controls::ALL_MAIL.to_owned(),
        Window::Days(days) => copy::controls::last_days(days),
    }
}

/// What Remove from Server calls `remove`.
pub(super) fn remove_label(remove: Remove) -> String {
    match remove {
        Remove::Never => copy::controls::NEVER.to_owned(),
        Remove::AfterDays(days) => copy::controls::after_days(days),
        Remove::WhenDeleted => copy::controls::WHEN_DELETED_HERE.to_owned(),
    }
}

/// The menu's items: the choices offered, and the current one too where a hand-edited file holds
/// one they do not include, so the menu still names what is kept.
fn items<T: Copy + PartialEq + 'static>(
    choices: &[T],
    current: T,
    label: fn(T) -> String,
) -> Vec<MenuItem<T>> {
    let mut all: Vec<T> = choices.to_vec();
    if !all.contains(&current) {
        all.push(current);
    }
    all.into_iter()
        .map(|one| MenuItem::new(one, label(one)))
        .collect()
}

/// Whether picking `chosen` for Remove from Server must be confirmed first: it deletes mail from
/// the server, which nothing here can bring back, so any choice but Never, other than the one
/// already kept, asks.
fn asks_first(kept: Remove, chosen: Remove) -> bool {
    chosen != Remove::Never && chosen != kept
}

/// What the confirmation of `chosen` says.
fn asked(chosen: Remove) -> String {
    match chosen {
        Remove::AfterDays(days) => copy::confirm::remove_from_server_after(days),
        Remove::Never | Remove::WhenDeleted => {
            copy::confirm::REMOVE_FROM_SERVER_WHEN_DELETED.to_owned()
        }
    }
}

/// What the rows keep: the setting as it was last written, why a write failed, and the revision
/// a write moves.
#[derive(Clone, Copy)]
struct Held {
    kept: Signal<Kept>,
    failed: Signal<Option<String>>,
}

/// Write the setting in `dirs` as `edit` leaves it and keep what that made, or say why it could
/// not be written.
fn save(dirs: Option<&WindowDirs>, held: Held, edit: impl Fn(Kept) -> Kept) {
    let Held {
        mut kept,
        mut failed,
    } = held;
    let saved = match dirs {
        Some(dirs) => offline::change(&dirs.config, &edit).map_err(String::from),
        None => Err(copy::errors::NO_CONFIG_DIRECTORY.to_owned()),
    };
    match saved {
        Ok(()) => {
            let now = edit(kept.peek().clone());
            kept.set(now);
            failed.set(None);
        }
        Err(why) => failed.set(Some(why)),
    }
}

/// The account's rows. Mail kept only here has none: it has no server (the caller's to know).
#[component]
pub(super) fn Syncing(row: AccountRow) -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let id = row.id.clone();
    let kept = use_signal({
        let dirs = dirs.clone();
        move || {
            let loaded = dirs.as_ref().map(|dirs| offline::load(&dirs.config));
            // No config directory: every account at the defaults, which a choice then says it
            // cannot keep.
            loaded.unwrap_or_default()
        }
    });
    let failed = use_signal(|| None::<String>);
    let mut asking = use_signal(|| None::<Remove>);
    let held = Held { kept, failed };
    let (every, window, remove) = {
        let kept = kept.read();
        (kept.every_of(&id), kept.window_of(&id), kept.remove_of(&id))
    };
    let pop3 = matches!(row.plan.incoming, Incoming::Pop3 { .. });
    let (on_every, on_window, on_remove) = (id.clone(), id.clone(), id);
    let (dirs_every, dirs_window, dirs_remove) = (dirs.clone(), dirs.clone(), dirs.clone());
    let (confirm_remove, confirm_dirs) = (on_remove.clone(), dirs);
    rsx! {
        FieldRow { label: copy::fields::CHECK_FOR_NEW_MAIL,
            PopUpButton::<Every> {
                items: items(&Every::CHOICES, every, every_label),
                value: Some(every),
                common: Common { aria_label: Some(copy::fields::CHECK_FOR_NEW_MAIL.to_owned()), ..Common::default() },
                onpick: move |one: Every| {
                    save(dirs_every.as_ref(), held, |kept: Kept| kept.with_every(on_every.clone(), one));
                },
            }
        }
        FieldRow { label: copy::fields::DOWNLOAD_MAIL_FROM,
            PopUpButton::<Window> {
                items: items(&Window::CHOICES, window, window_label),
                value: Some(window),
                common: Common { aria_label: Some(copy::fields::DOWNLOAD_MAIL_FROM.to_owned()), ..Common::default() },
                onpick: move |one: Window| {
                    save(dirs_window.as_ref(), held, |kept: Kept| kept.with_window(on_window.clone(), one));
                },
            }
        }
        if pop3 {
            FieldRow { label: copy::fields::REMOVE_FROM_SERVER,
                PopUpButton::<Remove> {
                    items: items(&Remove::CHOICES, remove, remove_label),
                    value: Some(remove),
                    common: Common { aria_label: Some(copy::fields::REMOVE_FROM_SERVER.to_owned()), ..Common::default() },
                    onpick: move |one: Remove| {
                        if asks_first(remove, one) {
                            asking.set(Some(one));
                        } else {
                            save(dirs_remove.as_ref(), held, |kept: Kept| kept.with_remove(on_remove.clone(), one));
                        }
                    },
                }
            }
        }
        if let Some(chosen) = asking() {
            Alert {
                title: copy::confirm::REMOVE_FROM_SERVER.to_owned(),
                message: Some(TextLine::from(asked(chosen))),
                style: AlertStyle::Critical,
                buttons: vec![
                    AlertButton::new(copy::controls::REMOVE, AlertRole::Destructive, EventHandler::new(move |()| {
                        asking.set(None);
                        save(confirm_dirs.as_ref(), held, |kept: Kept| kept.with_remove(confirm_remove.clone(), chosen));
                    })),
                    AlertButton::new(
                        copy::controls::CANCEL,
                        AlertRole::Cancel,
                        EventHandler::new(move |()| asking.set(None)),
                    ),
                ],
            }
        }
        if let Some(why) = failed() {
            Problem {
                title: copy::titles::NOT_SAVED,
                why,
                onclose: move |()| {
                    let mut failed = failed;
                    failed.set(None);
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "syncing_tests.rs"]
mod tests;
