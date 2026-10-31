use dioxus::prelude::*;
use mail_client::Client;
use mail_client::Syncing;
use mail_domain::*;
use porter_core::AccountId;

/// How many rows the list pane asks for at a time.
pub const PAGE: u32 = 100;

/// The account's name as the window writes it: the description the person gave it, else its
/// address, or "Local folders".
pub fn named(account: &mail_client::query::Account) -> String {
    if let Some(description) = &account.description {
        description.clone()
    } else if account.local {
        LOCAL_FOLDERS.to_owned()
    } else {
        account.address.clone()
    }
}

/// One account as the sidebar draws it: the id, the address, and the plan the provider comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub id: AccountId,
    pub address: String,
    pub plan: AccountPlan,
    /// What the person calls it, written in place of the address where the window names it.
    pub description: Option<String>,
    /// Whether the person paused its syncing.
    pub syncing: Syncing,
}

/// Every account, oldest first. A plan that does not parse becomes a plain IMAP account so the
/// account still has a host to name; `'{}'` is what the oldest fixtures wrote.
pub fn account_rows(client: &Client) -> Vec<AccountRow> {
    client
        .stored()
        .list_accounts()
        .unwrap_or_default()
        .into_iter()
        .map(|account| {
            let plan = account
                .plan
                .unwrap_or_else(|_| fallback_plan(&account.address));
            // An account whose preferences cannot be read is shown as it was before there were
            // any: by its address, syncing.
            let prefs = client
                .stored()
                .account_prefs(account.id.clone())
                .unwrap_or_default();
            AccountRow {
                id: account.id,
                address: account.address,
                plan,
                description: prefs.description,
                syncing: prefs.syncing,
            }
        })
        .collect()
}

/// Every account as a row, from the accounts the client keeps read ([`mail_client::query::Accounts`],
/// which carries each one's preferences): a component draws from it without asking the store, and
/// draws again when an account, its folders or its preferences change. Empty until the client's
/// first read, which is also what a read that failed shows.
pub fn use_account_rows() -> Memo<Vec<AccountRow>> {
    let every = crate::live::use_live(|| mail_client::query::Accounts);
    use_memo(move || {
        every
            .read()
            .value()
            .map(|accounts| rows_of(accounts))
            .unwrap_or_default()
    })
}

/// [`account_rows`], from the accounts the client keeps read: no store is asked.
pub fn rows_of(accounts: &[mail_client::query::Account]) -> Vec<AccountRow> {
    accounts
        .iter()
        .map(|account| AccountRow {
            id: account.id.clone(),
            address: account.address.clone(),
            plan: account
                .plan
                .clone()
                .unwrap_or_else(|| fallback_plan(&account.address)),
            description: account.description.clone(),
            syncing: if account.paused {
                Syncing::Paused
            } else {
                Syncing::default()
            },
        })
        .collect()
}

impl AccountRow {
    /// Whether this is local folders: mail kept on this computer, never synced, never sent from.
    /// The same test as [`mail_client::sync::local_accounts`].
    pub fn is_local(&self) -> bool {
        !self.plan.incoming.caps().has_server
    }

    /// The account's name as the window writes it: the description the person gave it, else
    /// its address, or "Local folders".
    pub fn shown(&self) -> String {
        if let Some(description) = &self.description {
            description.clone()
        } else if self.is_local() {
            LOCAL_FOLDERS.to_owned()
        } else {
            self.address.clone()
        }
    }
}

/// Whether nothing the list shows has a server, so there is nothing for a sync to do: every
/// account in the Space's `scope` is local folders, or there is no account at all. The sidebar then has no sync mark and
/// the list's ⋯ no Sync Now.
pub fn syncs_nothing(rows: &[AccountRow], scope: &crate::space::Scope) -> bool {
    rows.iter()
        .filter(|row| scope.shows(row.id.clone()))
        .all(|row| row.is_local())
}

/// What the window calls the local-only account, wherever it names it.
pub const LOCAL_FOLDERS: &str = "Local folders";

fn fallback_plan(address: &str) -> AccountPlan {
    AccountPlan {
        address: address.to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    }
}

/// Every configured account, or why the client could not say. Where an empty answer would be
/// acted on (taking accounts out of the Spaces), a failed read must not look like no accounts.
#[cfg(any(test, feature = "testing"))]
pub fn known_accounts(client: &Client) -> Result<Vec<AccountId>, mail_client::prelude::StoreError> {
    Ok(client
        .stored()
        .list_accounts()?
        .into_iter()
        .map(|account| account.id)
        .collect())
}

/// Every configured account, for the places that are not scoped to one.
pub fn accounts(client: &Client) -> Vec<AccountId> {
    account_rows(client).into_iter().map(|row| row.id).collect()
}
