//! An account's own page in Settings' Accounts pane, pushed over the list as System Settings
//! pushes an Internet Account: its servers and sign-in, and Remove Account….
//!
//! The settings are read, not edited: changing them is the Add account sheet's. Remove asks first
//! in quire's alert, naming how much mail goes and whether the server still has it. Only the
//! confirming button removes; Escape and Cancel leave the page as it was. A removal runs off the
//! thread that draws (the keyring blocks), on the window's own task, so turning to another page
//! does not cut it short. When it is done the offline setting forgets the account, the revision
//! moves and the pane goes back to the list: the links, the lists, the Spaces and the pressed tile
//! follow it, as they do for an account removed with `mailo account remove`. What the page says
//! is [`words`]'s.

mod words;

use crate::ui::appearance::WindowDirs;
use crate::ui::data::{AccountRow, account_rows};
use crate::ui::press::on_primary;
use crate::ui::view::{AccountStep, AccountsPage, Shell};
use dioxus::prelude::*;
use ds::components::content::label::Label;
use ds::components::controls::button_model::ButtonRole;
use ds::components::fields::field_row::FieldRow;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use mail_runtime::AccountSecrets;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::sync::Arc;

/// What the button that asks says.
const ASK: &str = "Remove Account\u{2026}";

/// The keyring a removal forgets the sign-in from, handed in as a context so tests reach their
/// own and never the user's.
#[derive(Clone)]
pub(in crate::ui) struct Seams {
    pub secrets: Arc<dyn AccountSecrets>,
    /// Where the sign-ins Mail holds itself are kept, for removing an account it signed in itself
    /// while linked to accountd (`secrets` is then the link's, which holds none of them).
    pub own: Arc<dyn AccountSecrets>,
}

impl Seams {
    /// The system keyring. In this crate's tests, an empty keyring of its own.
    #[cfg(test)]
    fn real() -> Seams {
        Seams {
            secrets: Arc::new(porter_secrets::MemorySecrets::default()),
            own: Arc::new(porter_secrets::MemorySecrets::default()),
        }
    }

    #[cfg(not(test))]
    fn real() -> Seams {
        Seams {
            secrets: crate::edge::secrets(),
            own: crate::edge::own_secrets(),
        }
    }
}

fn seams() -> Seams {
    try_consume_context::<Seams>().unwrap_or_else(Seams::real)
}

/// The account's row, read from the store. `None` once it is gone.
pub(super) fn row_of(account: &AccountId) -> Option<AccountRow> {
    let store = try_consume_context::<Arc<SqliteStore>>()?;
    account_rows(&store)
        .into_iter()
        .find(|row| row.id == *account)
}

/// Move the page to `step`.
fn to(mut shell: Signal<Shell>, step: AccountStep) {
    shell.write().accounts_pane.step = step;
}

/// Back one page, as the back button, Escape and ⌘[ ask: from an account's page to the list.
/// Nothing while the question is up (the alert answers Escape itself) or a removal is under way.
pub(super) fn back(mut shell: Signal<Shell>) {
    let step = shell.peek().accounts_pane.step.clone();
    match step {
        AccountStep::Asking | AccountStep::Removing => {}
        AccountStep::Showing | AccountStep::Refused(_) => {
            let mut write = shell.write();
            let popped = write.accounts_pane.path.popped();
            write.accounts_pane.path = popped;
            write.accounts_pane.step = AccountStep::Showing;
        }
    }
}

/// The confirming button: remove the account, then tidy what the window kept of it and go back
/// to the list.
fn confirm(mut shell: Signal<Shell>, mut revision: Signal<u64>, account: AccountId) {
    if shell.peek().accounts_pane.step != AccountStep::Asking {
        return;
    }
    let Some(store) = try_consume_context::<Arc<SqliteStore>>() else {
        return;
    };
    to(shell, AccountStep::Removing);
    let seams = seams();
    let dirs = try_consume_context::<WindowDirs>();
    let removing = account.clone();
    // The secrets are forgotten through porter's async trait, which does its work on a runtime
    // of its own (`mail_runtime::account_secrets`), so this task awaits it as it is; the
    // database half is quick and the store's own. The task is the window's, not the page's: a
    // page left mid-removal would otherwise drop it between the keyring and the database.
    dioxus::core::spawn_forever(async move {
        match mail_core::account::remove(&store, seams.secrets.as_ref(), removing).await {
            Ok(_) => {
                forgotten(dirs.as_ref(), account.clone());
                revision += 1;
                let mut write = shell.write();
                write.accounts_pane.step = AccountStep::Showing;
                if *write.accounts_pane.path.current() == AccountsPage::Account(account) {
                    let popped = write.accounts_pane.path.popped();
                    write.accounts_pane.path = popped;
                }
            }
            Err(error) => to(shell, AccountStep::Refused(words::refused(&error))),
        }
    });
}

/// What the window kept of a removed account that the Spaces do not hold: the offline setting.
/// The Spaces and the pressed tile follow the revision (`app`'s effect), as they do for an
/// account removed from a terminal.
fn forgotten(dirs: Option<&WindowDirs>, account: AccountId) {
    if let Some(dirs) = dirs {
        let _ = mail_core::offline::save(&dirs.config, account, mail_core::offline::Keep::Bodies);
    }
}

/// What asking to remove `account`, an account Mail signed in itself that is set aside while
/// linked to accountd, says: the title, the body and the confirming button. The words of any
/// removal: they name the mail on this computer that goes with it.
pub(super) fn held_asking(account: &AccountId, address: &str) -> (String, String, &'static str) {
    let store = try_consume_context::<Arc<SqliteStore>>();
    let held = store
        .as_ref()
        .and_then(|store| store.offline(account.clone()).ok())
        .map_or(0, |offline| {
            usize::try_from(offline.messages).unwrap_or(usize::MAX)
        });
    let incoming = store
        .as_ref()
        .and_then(|store| store.account(account.clone()).ok().flatten())
        .and_then(|stored| stored.plan.ok())
        .map(|plan| plan.incoming);
    let asked = words::asking(
        address,
        held,
        incoming.as_ref().unwrap_or(&mail_domain::Incoming::Graph),
    );
    (asked.title, asked.body, asked.confirm)
}

/// Remove `account`, which Mail signed in itself and which is set aside while linked: the same
/// removal as `mailo account remove`, forgetting its sign-in from mailo's own store. `failed` says
/// why when it did not happen; the revision moves when it did.
pub(super) fn remove_held(
    mut revision: Signal<u64>,
    mut failed: Signal<Option<String>>,
    account: AccountId,
) {
    let Some(store) = try_consume_context::<Arc<SqliteStore>>() else {
        return;
    };
    let seams = seams();
    let dirs = try_consume_context::<WindowDirs>();
    dioxus::core::spawn_forever(async move {
        match mail_core::account::remove(&store, seams.own.as_ref(), account.clone()).await {
            Ok(_) => {
                forgotten(dirs.as_ref(), account);
                failed.set(None);
                revision += 1;
            }
            Err(error) => failed.set(Some(words::refused(&error))),
        }
    });
}

/// The account's page, under the pane's header, which names it.
#[component]
pub(super) fn AccountPage(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    account: AccountId,
) -> Element {
    // Read again when an account comes or goes.
    let _ = revision();
    let Some(row) = row_of(&account) else {
        // Removed from elsewhere while the page was shown.
        return rsx! {
            Form {
                FormSection {
                    footer: Some(words::refused(&mail_core::account::RemoveError::Unknown)),
                }
            }
        };
    };
    let step = shell.read().accounts_pane.step.clone();
    let name = row.shown();
    let refused = match &step {
        AccountStep::Refused(why) => Some(why.clone()),
        AccountStep::Showing | AccountStep::Asking | AccountStep::Removing => None,
    };
    let asking = (step == AccountStep::Asking).then(|| {
        let held = try_consume_context::<Arc<SqliteStore>>()
            .and_then(|store| store.offline(row.id.clone()).ok())
            .map_or(0, |offline| {
                usize::try_from(offline.messages).unwrap_or(usize::MAX)
            });
        words::asking(&row.address, held, &row.plan.incoming)
    });
    let removing = if step == AccountStep::Removing {
        Availability::Busy
    } else {
        Availability::Enabled
    };
    rsx! {
        Form {
            FormSection { title: Some("Servers".to_owned()),
                for line in words::settings(&row.plan) {
                    FieldRow { key: "{line.label}", label: line.label,
                        Label { text: line.value }
                    }
                }
            }
            div { class: "acct-actions",
                if let Some(why) = refused {
                    Label { text: why, severity: Some(Severity::Warn) }
                }
                Button {
                    label: ASK,
                    role: ButtonRole::Destructive,
                    common: Common { aria_label: Some(format!("Remove {name}")), ..Common::default() },
                    availability: removing,
                    onclick: on_primary(move || to(shell, AccountStep::Asking)),
                }
            }
        }
        if let Some(asked) = asking {
            Alert {
                title: asked.title.clone(),
                message: Some(TextLine::from(asked.body.clone())),
                style: AlertStyle::Critical,
                buttons: vec![
                    AlertButton::new(asked.confirm, AlertRole::Destructive, EventHandler::new({
                        let account = account.clone();
                        move |()| confirm(shell, revision, account.clone())
                    })),
                    AlertButton::new(
                        "Cancel",
                        AlertRole::Cancel,
                        EventHandler::new(move |()| to(shell, AccountStep::Showing)),
                    ),
                ],
            }
        }
    }
}

#[cfg(test)]
mod tests;
