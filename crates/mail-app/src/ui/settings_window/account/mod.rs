//! An account's own page in Settings' Accounts pane, pushed over the list as System Settings
//! pushes an Internet Account: whether all its mail is downloaded ([`super::offline`]), the name
//! it sends as and its aliases ([`sends_as`]), its servers and sign-in, its own keys and
//! certificates (`pgp::keys`), and, last and alone, Remove Account….
//!
//! The servers are read, not edited: changing them is the Add account sheet's. Remove asks first
//! in quire's alert, naming how much mail goes and whether the server still has it. Only the
//! confirming button removes; Escape and Cancel leave the page as it was. A removal runs off the
//! thread that draws (the keyring blocks), on the window's own task, so turning to another page
//! does not cut it short. When it is done the offline setting forgets the account, the revision
//! moves and the pane goes back to the list: the links, the lists, the Spaces and the pressed tile
//! follow it, as they do for an account removed with `mailo account remove`. What the page says
//! is [`words`]'s.

mod sends_as;
mod words;

use crate::ui::appearance::WindowDirs;
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::data::{AccountRow, account_rows};
use crate::ui::fetching::{Need, Needing, use_needs};
use crate::ui::pgp::keys::Whose;
use crate::ui::press::on_primary;
use crate::ui::reauth::SIGN_IN;
use crate::ui::view::{AccountStep, AccountsPage, Shell};
use dioxus::prelude::*;
use ds::components::content::label::Label;
use ds::components::controls::button_model::ButtonRole;
use ds::components::fields::field_row::FieldRow;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_core::AccountSecrets;
use mail_core::{SqliteStore, Store};
use porter_core::AccountId;
use std::sync::Arc;

/// What the button that asks says.
const ASK: &str = "Remove Account\u{2026}";

/// The keyring a removal forgets the sign-in from, handed in as a context so tests reach their
/// own and never the user's.
#[derive(Clone)]
pub(in crate::ui) struct Seams {
    pub secrets: Arc<dyn AccountSecrets>,
}

impl Seams {
    /// The system keyring: the launched window's, handed as a root context there
    /// (`launch::native`) and nowhere else.
    pub(in crate::ui) fn real() -> Seams {
        Seams {
            secrets: crate::edge::secrets(),
        }
    }

    /// A window's whose root handed none, as a test's does: a keyring that holds nothing, so a
    /// removal forgets nothing and touches no keyring of the user's.
    fn none() -> Seams {
        Seams {
            secrets: Arc::new(mail_core::NoKeyring),
        }
    }
}

fn seams() -> Seams {
    try_consume_context::<Seams>().unwrap_or_else(Seams::none)
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

/// Show the list: the account's page has nothing left to show. A removal under way goes back
/// to it on its own.
fn to_list(mut shell: Signal<Shell>) {
    if shell.peek().accounts_pane.step == AccountStep::Removing {
        return;
    }
    let mut write = shell.write();
    write.accounts_pane.path = PanePath::new(AccountsPage::List);
    write.accounts_pane.step = AccountStep::Showing;
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

/// Sign `account` in again, as the dialog in the first window does; that window is told, so that
/// it tries the account again when the sign-in is done.
fn sign_in(address: &str, account: &AccountId, need: Need) {
    if let Some(needing) = try_consume_context::<Needing>() {
        needing.signing_in(account.clone());
    }
    match need {
        Need::SignIn => crate::ui::add_account::again(address.to_owned()),
        Need::Allow => crate::ui::add_account::allow_again(),
    }
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
    // Whether the account waits for the person: the first window knows, and says so to this one.
    let waiting = use_needs();
    let row = row_of(&account);
    // Removed from elsewhere while the page was shown: the pane goes back to the list.
    let gone = row.is_none();
    use_effect(use_reactive!(|gone| {
        if gone {
            to_list(shell);
        }
    }));
    let Some(row) = row else {
        return rsx! {};
    };
    let step = shell.read().accounts_pane.step.clone();
    let name = row.shown();
    let need = waiting
        .read()
        .iter()
        .find(|(id, _)| *id == account)
        .map(|(_, need)| *need);
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
            if let Some(need) = need {
                FormSection {
                    FieldRow { label: copy::fields::STATUS,
                        div { class: "doctor-acts",
                            Label { text: words::WAITING }
                            Button {
                                label: SIGN_IN,
                                size: ControlSize::Small,
                                common: Common { aria_label: Some(format!("{SIGN_IN} to {name}")), ..Common::default() },
                                onclick: on_primary({
                                    let (address, account) = (row.address.clone(), account.clone());
                                    move || sign_in(&address, &account, need)
                                }),
                            }
                        }
                    }
                }
            }
            if !row.is_local() {
                FormSection {
                    super::offline::DownloadAll { row: row.clone() }
                }
            }
            if !row.is_local() {
                sends_as::SendsAs { account: account.clone(), revision }
            }
            FormSection { title: Some(copy::headers::SERVERS.to_owned()),
                for line in words::settings(&row.plan) {
                    FieldRow { key: "{line.label}", label: line.label,
                        Label { text: line.value }
                    }
                }
            }
            if !row.is_local() {
                crate::ui::pgp::keys::KeysPage { whose: Whose::Account(row.address.clone()), revision }
            }
            // Removing is the page's last part, alone and outside any group, as System Settings
            // puts it: it cannot be undone, so it asks first.
            div { class: "acct-actions",
                Button {
                    label: ASK,
                    role: ButtonRole::Destructive,
                    common: Common { aria_label: Some(format!("Remove {name}")), ..Common::default() },
                    availability: removing,
                    onclick: on_primary(move || to(shell, AccountStep::Asking)),
                }
            }
        }
        if let Some(why) = refused {
            Problem { title: copy::titles::NOT_REMOVED, why, onclose: move |()| to(shell, AccountStep::Showing) }
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
                        copy::controls::CANCEL,
                        AlertRole::Cancel,
                        EventHandler::new(move |()| to(shell, AccountStep::Showing)),
                    ),
                ],
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests;
