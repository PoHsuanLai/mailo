//! An account's own page in Settings' Accounts pane, pushed over the list as System Settings
//! pushes an Internet Account: whether all its mail is downloaded ([`super::offline`]), how often
//! it is checked and how far back ([`super::syncing`]), the name it sends as and its aliases
//! ([`sends_as`]), its servers and sign-in, its own keys and certificates (`pgp::keys`), and,
//! last and alone, Remove Account….
//!
//! What the account is called, whether it syncs and its folders' roles are changed here
//! ([`edit`]). Under the servers, how much of the account's storage its server says is used,
//! where the server keeps a quota (IMAP's `QUOTA`, JMAP's quotas: `mail_core::SyncOps::quota`). It
//! is asked once as the page opens, off the thread that draws, and the row is left out until the
//! server answers, and for good when it cannot or keeps none: the page is about the account, and
//! a server that is slow or away must not hold it up or fill it with an error.
//!
//! The servers are read, not edited: changing them is porter's server editor's, which the page
//! will offer once mailo is on a porter that has it. Remove asks first in quire's alert, naming
//! how much mail goes and whether the server still has it. Only the
//! confirming button removes; Escape and Cancel leave the page as it was. A removal runs off the
//! thread that draws (the keyring blocks), on the window's own task, so turning to another page
//! does not cut it short. When it is done the offline setting forgets the account, the store
//! has moved and the pane goes back to the list: the links, the lists, the Spaces and the pressed
//! tile follow it, as they do for an account removed with `mailo account remove`. What the page
//! says is [`words`]'s.

mod edit;
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
use mail_client::Storage;
use mail_client::prelude::AccountSecrets;
use porter_core::AccountId;
use std::sync::Arc;

/// What the button that asks says.
const ASK: &str = "Remove Account\u{2026}";

/// Ask an account's server how much of its storage is used: [`mail_core::SyncOps::quota`], or
/// why it could not say. Blocks, so it is run off the thread that draws.
pub(in crate::ui) type ReadStorage =
    Arc<dyn Fn(mail_client::Client, AccountId) -> Result<Option<Storage>, String> + Send + Sync>;

/// The keyring a removal forgets the sign-in from, and what asks a server how full the account
/// is, handed in as a context so tests reach their own and never the user's.
#[derive(Clone)]
pub(in crate::ui) struct Seams {
    pub secrets: Arc<dyn AccountSecrets>,
    pub storage: ReadStorage,
}

impl Seams {
    /// The system keyring: the launched window's, handed as a root context there
    /// (`launch::native`) and nowhere else.
    pub(in crate::ui) fn real(client: &mail_client::Client) -> Seams {
        Seams {
            secrets: client.net().secrets(),
            storage: remembered(
                Arc::new(|client, account| {
                    client.net().quota(account).map_err(crate::ui::remedy::told)
                }),
                STORAGE_KEPT,
            ),
        }
    }

    /// A window's whose root handed none, as a test's does: a keyring that holds nothing, so a
    /// removal forgets nothing and touches no keyring of the user's, and no server to ask, so
    /// the page says nothing of storage and opens no socket.
    fn none() -> Seams {
        Seams {
            secrets: Arc::new(mail_client::prelude::NoKeyring),
            storage: Arc::new(|_, _| Ok(None)),
        }
    }
}

/// How long a server's answer about an account's storage is kept: the page is opened again and
/// again, and each ask is a connection and a sign-in.
const STORAGE_KEPT: std::time::Duration = std::time::Duration::from_secs(300);

/// How long the page waits for a server to say how full the account is before it says nothing of
/// storage: a server that does not answer is not a reason to hold a thread, or the row, for good.
const STORAGE_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// `ask`, but an answer is kept for `kept` and handed back to the next ask for the same account.
/// A server that failed to answer is asked again.
fn remembered(ask: ReadStorage, kept: std::time::Duration) -> ReadStorage {
    type Told = std::collections::HashMap<AccountId, (std::time::Instant, Option<Storage>)>;
    let told: Arc<std::sync::Mutex<Told>> = Arc::default();
    Arc::new(move |client, account| {
        let now = std::time::Instant::now();
        let held = told
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&account)
            .filter(|(at, _)| now.saturating_duration_since(*at) < kept)
            .map(|(_, storage)| *storage);
        if let Some(storage) = held {
            return Ok(storage);
        }
        let storage = ask(client, account.clone())?;
        told.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(account, (now, storage));
        Ok(storage)
    })
}

/// What `ask` says of `account`'s storage, or nothing once `wait` has passed. The server's own
/// thread is not stopped, only left behind.
async fn asked_within(
    wait: std::time::Duration,
    ask: ReadStorage,
    client: mail_client::Client,
    account: AccountId,
) -> Option<Storage> {
    let asked = tokio::task::spawn_blocking(move || ask(client, account));
    let Ok(done) = tokio::time::timeout(wait, asked).await else {
        log::debug!("no storage for the account page: the server did not answer in time");
        return None;
    };
    done.ok()?
        // Said nowhere: the row is left out, and the page is no place for a server's trouble,
        // which the account's own status says when it matters.
        .unwrap_or_else(|why| {
            log::debug!("no storage for the account page: {why}");
            None
        })
}

fn seams() -> Seams {
    try_consume_context::<Seams>().unwrap_or_else(Seams::none)
}

/// The account's row, read from the client. `None` once it is gone.
pub(super) fn row_of(account: &AccountId) -> Option<AccountRow> {
    let client = crate::ui::live::try_client()?;
    account_rows(&client)
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
fn confirm(mut shell: Signal<Shell>, account: AccountId) {
    if shell.peek().accounts_pane.step != AccountStep::Asking {
        return;
    }
    let Some(client) = crate::ui::live::try_client() else {
        return;
    };
    to(shell, AccountStep::Removing);
    let seams = seams();
    let dirs = try_consume_context::<WindowDirs>();
    let removing = account.clone();
    // The client forgets the secrets and then the account's mail, waiting on its runtime, so a
    // blocking thread waits for it. The task is the window's, not the page's: a page left
    // mid-removal would otherwise drop it between the keyring and the database.
    dioxus::core::spawn_forever(async move {
        let removed = tokio::task::spawn_blocking(move || {
            client
                .net()
                .remove_account(seams.secrets.as_ref(), removing)
        })
        .await;
        // A thread that panicked removed nothing it can say.
        let Ok(removed) = removed else {
            return;
        };
        match removed {
            Ok(_) => {
                forgotten(dirs.as_ref(), account.clone());
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

/// What the window kept of a removed account that the Spaces do not hold: how it was synced.
/// The Spaces and the pressed tile follow the store (`app`'s effect), as they do for an
/// account removed from a terminal.
fn forgotten(dirs: Option<&WindowDirs>, account: AccountId) {
    if let Some(dirs) = dirs {
        // The account is gone either way; a setting left behind is never asked about.
        let _ = mail_client::offline::change(&dirs.config, |kept| kept.without(account));
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

/// How much of `account`'s storage its server says is used, once it has said: `None` until then,
/// and for an account whose server keeps no quota, cannot be reached, or has none to ask.
///
/// Not asked of an account that waits for the person to sign in again (that would be a sign-in
/// attempt for nothing, each time the page opens), nor of one whose syncing is paused: a paused
/// account is left alone by every connection mailo makes.
fn use_storage(account: AccountId, waiting: Signal<Vec<(AccountId, Need)>>) -> Option<Storage> {
    let read = use_resource(use_reactive!(|account| {
        let waits = waiting.read().iter().any(|(id, _)| *id == account);
        let asked = row_of(&account)
            .is_some_and(|row| {
                !waits
                    && row.syncing != mail_client::Syncing::Paused
                    && row.plan.incoming.caps().has_quota
            })
            .then(crate::ui::live::try_client)
            .flatten();
        let ask = seams().storage;
        async move { asked_within(STORAGE_WAIT, ask, asked?, account).await }
    }));
    (*read.read()).flatten()
}

/// The account's page, under the pane's header, which names it.
#[component]
pub(super) fn AccountPage(shell: Signal<Shell>, account: AccountId) -> Element {
    // Read again when an account comes or goes.
    let _ = crate::ui::live::moved_for(mail_client::Topics::ACCOUNTS);
    // Whether the account waits for the person: the first window knows, and says so to this one.
    let waiting = use_needs();
    let storage = use_storage(account.clone(), waiting);
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
        let held = crate::ui::live::try_client()
            .and_then(|client| client.stored().offline(row.id.clone()).ok())
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
                    super::syncing::Syncing { row: row.clone() }
                }
            }
            if !row.is_local() {
                edit::About { row: row.clone() }
                sends_as::SendsAs { account: account.clone() }
            }
            FormSection { title: Some(copy::headers::SERVERS.to_owned()),
                for line in words::settings(&row.plan).into_iter().chain(storage.as_ref().map(words::storage)) {
                    FieldRow { key: "{line.label}", label: line.label,
                        Label { text: line.value }
                    }
                }
            }
            if !row.is_local() {
                edit::FolderRoles { row: row.clone() }
            }
            if matches!(row.plan.incoming, mail_domain::Incoming::Graph) {
                FormSection {
                    super::push_relay::PushRelay { row: row.clone() }
                }
            }
            if !row.is_local() {
                crate::ui::pgp::keys::KeysPage { whose: Whose::Account(row.address.clone()) }
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
                        move |()| confirm(shell, account.clone())
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
