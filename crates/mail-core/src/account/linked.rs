//! Accounts that are the desktop accountd's (porter, step E6).
//!
//! mailo holds a grant on each, and its store row says so ([`AuthPlan::Granted`]): the plan keeps
//! accountd's name for the account, the grant, and the servers the grant's candidate listed. No
//! password or token is written anywhere on this side: the engines are handed streams the daemon
//! has signed in (`mail_runtime::link`).
//!
//! The row's id stays mailo's own, a UUID, because every table of the store reads its account
//! column as one; accountd's id is in the plan, and [`find`] is how one is found by the other.
//!
//! Nothing here deletes or changes anything of the person's. An account mailo signed in itself is
//! not accountd's and is not made so: while linked it is set aside (`SqliteStore::set_granted_only`) and the
//! person adds it again through accountd. An address that is both (the person added it again while
//! mailo still holds it) cannot be two rows, the address being unique: accountd's is not added, and
//! the address is named in [`Reconciled::held`] until the person removes the old one.

use mail_domain::id::new_account_id;
use mail_domain::presets::{self, Manual, ManualPop3, Preset};
use mail_domain::{
    AccountPlan, Address, AuthPlan, HttpAuth, Identity, IdentityId, Incoming, IsDefault, Outgoing,
    Tls,
};
use mail_store::{NewAccount, SqliteStore};
use porter_core::{AccountId, Candidate, Family, ServiceEndpoint};

/// One account of accountd's, as the store has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    /// The row's id: mailo's.
    pub id: AccountId,
    /// accountd's name for it.
    pub account: AccountId,
    pub address: String,
}

/// What reading accountd's accounts did to the store.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reconciled {
    /// Addresses that were not here and now are.
    pub added: Vec<String>,
    /// Addresses whose grant or servers were read again and had changed.
    pub updated: Vec<String>,
    /// Addresses accountd offers that mailo already holds with a sign-in of its own: not added, the
    /// row being unique by address, and the old one left alone.
    pub held: Vec<String>,
    /// Accounts accountd offers that cannot be mail here, with why.
    pub unusable: Vec<(String, String)>,
}

fn tls_of(tls: porter_core::Tls) -> Tls {
    match tls {
        porter_core::Tls::Implicit => Tls::Implicit,
        porter_core::Tls::StartTls => Tls::StartTlsRequired,
        porter_core::Tls::Plain => Tls::Plaintext,
    }
}

fn server(endpoint: &ServiceEndpoint) -> (String, u16, Tls) {
    let origin = endpoint.url.origin();
    (origin.host, origin.port, tls_of(endpoint.tls))
}

/// The address the account is for: the login an endpoint wants when it is one, else its label.
fn address_of(candidate: &Candidate) -> Option<String> {
    candidate
        .endpoints
        .iter()
        .map(|e| e.login.0.as_str())
        .chain(std::iter::once(candidate.label.0.as_str()))
        .find(|name| name.contains('@') && !name.contains(char::is_whitespace))
        .map(str::to_lowercase)
}

/// What mailo makes of `candidate`: the plan and the starting capabilities.
///
/// The servers are the candidate's endpoints, read as mailo reads any: IMAP before JMAP before
/// Graph before POP3 for what arrives, SMTP for what leaves. An account whose grant lists none of
/// them is not mail here, and the error says so.
pub fn preset_of(
    candidate: &Candidate,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Preset, String> {
    let address = address_of(candidate)
        .ok_or_else(|| format!("accountd names no address for {}", candidate.label.0))?;
    let find = |family: Family| candidate.endpoints.iter().find(|e| e.family == family);
    let smtp = find(Family::Smtp).map(server);
    let mut preset = if let Some(imap) = find(Family::Imap) {
        let (host, port, tls) = server(imap);
        let (smtp_host, smtp_port, _) = smtp.clone().unwrap_or_else(|| (host.clone(), 465, tls));
        let mut preset = presets::manual(
            &address,
            &Manual {
                imap_host: host,
                imap_port: port,
                smtp_host,
                smtp_port,
                login: None,
            },
            now,
        );
        if let Incoming::Imap { tls: shown, .. } = &mut preset.plan.incoming {
            *shown = tls;
        }
        preset
    } else if let Some(jmap) = find(Family::Jmap) {
        presets::jmap(&address, jmap.url.as_str(), HttpAuth::Bearer)
    } else if find(Family::Graph).is_some() {
        presets::receive_through_graph(presets::microsoft_preset(&address, now))
    } else if let Some(pop3) = find(Family::Pop3) {
        let (host, port, _) = server(pop3);
        let (smtp_host, smtp_port, _) = smtp
            .clone()
            .unwrap_or_else(|| (host.clone(), 465, Tls::Implicit));
        presets::manual_pop3(
            &address,
            &ManualPop3 {
                pop3_host: host,
                pop3_port: port,
                smtp_host,
                smtp_port,
                login: None,
            },
            now,
        )
    } else {
        return Err(format!(
            "the grant on {address} lists no IMAP, POP3, JMAP or Graph server"
        ));
    };
    // What leaves: the account's SMTP server as it lists it, or nowhere when it lists none (and
    // the incoming side does not send for itself).
    match (&smtp, &preset.plan.incoming) {
        (Some((host, port, tls)), Incoming::Imap { .. } | Incoming::Pop3 { .. }) => {
            preset.plan.outgoing = Outgoing::Smtp {
                host: host.clone(),
                port: *port,
                tls: *tls,
            };
        }
        (None, Incoming::Imap { .. } | Incoming::Pop3 { .. }) => {
            preset.plan.outgoing = Outgoing::Nowhere;
        }
        _ => {}
    }
    preset.plan.auth = AuthPlan::Granted {
        account: candidate.account.clone(),
        grant: candidate.grant.clone(),
        endpoints: candidate.endpoints.clone(),
    };
    Ok(preset)
}

/// Every linked account in the store.
pub fn linked(store: &SqliteStore) -> Vec<Linked> {
    store
        .list_all_accounts()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|stored| {
            let AuthPlan::Granted { account, .. } = stored.plan.ok()?.auth else {
                return None;
            };
            Some(Linked {
                id: stored.id,
                account,
                address: stored.address,
            })
        })
        .collect()
}

/// The row of accountd's `account`, if mailo has one.
pub fn find(store: &SqliteStore, account: &AccountId) -> Option<Linked> {
    linked(store).into_iter().find(|l| l.account == *account)
}

/// The rows of the accounts whose object path ends in `segment` (`AccountRemoved` names an
/// account by its path only).
pub fn named_by_segment(store: &SqliteStore, segment: &str) -> Vec<Linked> {
    linked(store)
        .into_iter()
        .filter(|l| porter_core::object_segment(&l.account) == segment)
        .collect()
}

/// Reads `candidates` into the store: an account accountd offers that is not here is added, one
/// that is here has its grant and servers brought up to date, and nothing is removed (an account
/// that is no longer offered may have lost only the grant: that is [`forget`]'s to decide, on
/// accountd's `AccountRemoved`).
pub fn reconcile(
    store: &SqliteStore,
    candidates: &[Candidate],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Reconciled, String> {
    let mut said = Reconciled::default();
    for candidate in candidates {
        let preset = match preset_of(candidate, now) {
            Ok(preset) => preset,
            Err(why) => {
                said.unusable.push((candidate.label.0.clone(), why));
                continue;
            }
        };
        let address = preset.plan.address.clone();
        match find(store, &candidate.account) {
            Some(here) => {
                if refresh(store, &here, preset.plan)? {
                    said.updated.push(address);
                }
            }
            None => {
                if holds(store, &address) {
                    said.held.push(address);
                    continue;
                }
                insert(store, preset, now)?;
                said.added.push(address);
            }
        }
    }
    Ok(said)
}

/// Whether the store has a row for `address` that is not accountd's.
fn holds(store: &SqliteStore, address: &str) -> bool {
    store.account_by_address(address).ok().flatten().is_some()
}

fn insert(
    store: &SqliteStore,
    preset: Preset,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let id = new_account_id();
    let mut plan = preset.plan;
    plan.identities = vec![Identity {
        id: IdentityId::generate(),
        account: id.clone(),
        from: Address {
            name: None,
            email: plan.address.clone(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }];
    store
        .upsert_account(&NewAccount {
            id: &id,
            address: &plan.address,
            plan: &plan,
            caps: &preset.expected_caps,
            identities: &plan.identities,
            at: now,
        })
        .map_err(|e| format!("cannot save the account: {e}"))
}

/// Brings the row's grant and servers up to date, keeping what the person set (identities).
/// Whether anything changed.
fn refresh(store: &SqliteStore, here: &Linked, fresh: AccountPlan) -> Result<bool, String> {
    let stored = store
        .account(here.id.clone())
        .map_err(|e| format!("cannot read the account: {e}"))?
        .ok_or_else(|| "cannot read the account: it is gone".to_owned())?;
    let mut plan: AccountPlan = stored
        .plan
        .map_err(|e| format!("cannot read the plan: {}", crate::sync::why(&e)))?;
    let identities = std::mem::take(&mut plan.identities);
    let mut next = fresh;
    next.identities = identities;
    if next == plan_with(&plan, &next.identities) {
        return Ok(false);
    }
    store
        .set_account_plan(here.id.clone(), &next)
        .map_err(|e| format!("cannot save the account: {e}"))?;
    Ok(true)
}

fn plan_with(plan: &AccountPlan, identities: &[Identity]) -> AccountPlan {
    AccountPlan {
        identities: identities.to_vec(),
        ..plan.clone()
    }
}

/// Forgets what mailo kept of accounts accountd removed: their rows and their mail, and nothing
/// else. accountd holds the secrets, and has already dropped them and the grant.
pub fn forget(
    store: &SqliteStore,
    gone: &[Linked],
) -> Vec<(Linked, Result<mail_store::Freed, String>)> {
    gone.iter()
        .map(|account| {
            let freed = match store.remove_account(account.id.clone()) {
                Ok(Some(freed)) => Ok(freed),
                Ok(None) => Err("no such account".to_owned()),
                Err(e) => Err(e.to_string()),
            };
            (account.clone(), freed)
        })
        .collect()
}

#[cfg(test)]
#[path = "linked_tests.rs"]
mod tests;
