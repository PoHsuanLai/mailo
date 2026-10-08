//! The app's side of the link to the desktop's accountd (porter, step E6): the one start, the
//! first read, what follows `AccountAdded` and `AccountRemoved`, and the add.
//!
//! The decision is [`mail_runtime::link::start`]; what is the app's is asking the desktop whether
//! accountd is there (the capability probe of quire's `ds-desktop`, `Capability::Accounts`), which
//! a crate with no window and no quire cannot. The probe, and so the link, exist only in a build
//! with the `quire-desktop` feature on Linux; every other build is [`Link::Local`] and everything
//! here that is about accountd does nothing.
//!
//! Nothing is removed here that is the person's. Accounts accountd offers are read into the store
//! (`mail_core::account::reconcile`), an account it removes has its rows and mail forgotten here
//! and nothing else. Linked, the accounts Mail shows and syncs are accountd's alone: those Mail
//! signed in itself are set aside ([`hold_back`]), left exactly as they are for a start that is
//! not linked, and the account list says so in one line ([`HELD_LINE`]).

use std::sync::Arc;

use mail_core::account::{self, Linked, Reconciled};
use mail_runtime::link::{self, Accountd, Change, Here, Link};
use mail_store::SqliteStore;
use porter_core::consent::Usage;

/// What the follower of accountd's changes tells whoever draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    /// The accounts were read again.
    Read(Reconciled),
    /// Accounts accountd removed, forgotten here: their addresses.
    Forgotten(Vec<String>),
}

impl Heard {
    /// Whether the store is any different for it.
    pub fn changed(&self) -> bool {
        match self {
            Heard::Read(read) => !read.added.is_empty() || !read.updated.is_empty(),
            Heard::Forgotten(gone) => !gone.is_empty(),
        }
    }
}

/// The one decision, made once at the start of a process: asks the desktop whether accountd is
/// here, links to it when it is and this build can, and records the choice so every engine finds
/// it (`mail_runtime::platform_secrets`). `usage` is how the process uses its grants:
/// [`Usage::Background`] for `mailo watch`, [`Usage::Interactive`] for the window and the commands
/// someone is waiting on.
pub fn start(usage: Usage) -> Link {
    let here = probe();
    let link = with_runtime(link::start(here, usage));
    link::install(link.clone());
    link
}

/// Whether the desktop's account service answers: its bus name has an owner, or the bus would
/// start it.
#[cfg(all(feature = "quire-desktop", target_os = "linux"))]
fn probe() -> Here {
    with_runtime(async { here(&ds_desktop::Desktop::probe().await) })
}

/// A build without the desktop's extras, or off Linux: nothing to probe.
#[cfg(not(all(feature = "quire-desktop", target_os = "linux")))]
fn probe() -> Here {
    Here::No
}

/// What a probe's answer says of accountd.
#[cfg(all(feature = "quire-desktop", target_os = "linux"))]
pub fn here(desktop: &ds_desktop::Desktop) -> Here {
    if desktop.here(ds_desktop::Capability::Accounts) {
        Here::Yes
    } else {
        Here::No
    }
}

/// Wait for `work` on a runtime of its own: the callers here are the command line's and the
/// window's own threads, none of which is async.
fn with_runtime<T>(work: impl std::future::Future<Output = T>) -> T {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(work),
        // No runtime to be had is the process's own trouble; the link's futures need none of
        // mailo's, so they are driven by a parked thread, as the keyring's are.
        Err(_) => mail_runtime::block_on(work),
    }
}

/// Reads the accounts accountd offers Mail into the store. `None` when the link is not accountd.
pub fn read(store: &SqliteStore, link: &Link) -> Result<Option<Reconciled>, String> {
    let Some(accountd) = link.accountd() else {
        return Ok(None);
    };
    read_from(store, accountd).map(Some)
}

fn read_from(store: &SqliteStore, accountd: &Arc<dyn Accountd>) -> Result<Reconciled, String> {
    with_runtime(read_async(store, accountd))
}

/// [`read_from`] for a caller that is already on a runtime (the follower's).
async fn read_async(
    store: &SqliteStore,
    accountd: &Arc<dyn Accountd>,
) -> Result<Reconciled, String> {
    let candidates = accountd.candidates().await.map_err(|e| e.to_string())?;
    account::reconcile(store, &candidates, chrono::Utc::now())
}

/// A line for a person about what a read did, or none when nothing happened.
pub fn said(read: &Reconciled) -> Option<String> {
    let mut parts = Vec::new();
    if !read.added.is_empty() {
        parts.push(format!("added {}", read.added.join(", ")));
    }
    if !read.updated.is_empty() {
        parts.push(format!("updated {}", read.updated.join(", ")));
    }
    for address in &read.held {
        parts.push(format!(
            "{address} is still signed in by Mail itself, so the desktop's account of it is not \
             added. Remove it first: mailo account remove {address}"
        ));
    }
    for (name, why) in &read.unusable {
        parts.push(format!("{name}: {why}"));
    }
    (!parts.is_empty()).then(|| format!("the desktop's accounts: {}", parts.join("; ")))
}

/// Follows what accountd says changed, on a thread of its own, for as long as the link has anyone
/// to tell it: an account added (or one Mail was just allowed to use) is read in, one removed has
/// what mailo kept of it forgotten. `heard` is told after each. Nothing when the link is not
/// accountd's, or has no feed.
pub fn follow(
    store: &Arc<SqliteStore>,
    link: &Link,
    heard: impl Fn(Heard) + Send + 'static,
) -> Option<std::thread::JoinHandle<()>> {
    let accountd = link.accountd()?.clone();
    let store = store.clone();
    std::thread::Builder::new()
        .name("mailo-accountd".to_owned())
        .spawn(move || {
            with_runtime(async move {
                let Ok(Some(mut feed)) = accountd.changes().await else {
                    return;
                };
                while let Some(change) = feed.next().await {
                    if let Some(told) = react(&store, &accountd, change).await {
                        heard(told);
                    }
                }
            });
        })
        .ok()
}

/// What one change does to the store, and what to tell. Public to the tests of this crate.
pub(crate) async fn react(
    store: &SqliteStore,
    accountd: &Arc<dyn Accountd>,
    change: Change,
) -> Option<Heard> {
    match change {
        Change::Added | Change::Granted => match read_async(store, accountd).await {
            Ok(read) => Some(Heard::Read(read)),
            Err(why) => {
                eprintln!("mailo: reading the desktop's accounts: {why}");
                None
            }
        },
        Change::Removed(segment) => {
            let gone = account::named_by_segment(store, &segment);
            if gone.is_empty() {
                return None;
            }
            let mut forgotten = Vec::new();
            for (account, freed) in account::forget(store, &gone) {
                match freed {
                    Ok(_) => forgotten.push(account.address),
                    Err(why) => eprintln!("mailo: forgetting {}: {why}", account.address),
                }
            }
            Some(Heard::Forgotten(forgotten))
        }
    }
}

/// How an add through accountd's sheet ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// The person closed the sheet.
    Nothing,
    /// An account was added or signed in again, and read in.
    Account(Reconciled),
}

/// Opens accountd's add-account sheet (the desktop's shell draws it; mailo opens no window) and
/// reads in what was added. With an `address` that is one of accountd's, opens the sheet that
/// signs that account in again instead. Blocks until the sheet ends.
///
/// An account added from the sheet without "allow Mail to use it" is Mail's to ask for: the
/// chooser and consent sheet follow, as accountd's own, once.
pub fn add(
    store: &SqliteStore,
    accountd: &Arc<dyn Accountd>,
    address: Option<&str>,
) -> Result<Added, String> {
    use mail_runtime::link::LinkError;
    use porter_core::wire::Refusal;
    let ended = with_runtime(async {
        if let Some(address) = address
            && let Some(account) = account::linked_accounts(store)
                .into_iter()
                .find(|linked| linked.address.eq_ignore_ascii_case(address))
        {
            return accountd
                .reauthenticate(&account.account)
                .await
                .map(|()| None);
        }
        // An account already there is the one the person meant: Mail goes on to its grant, as for
        // one just added.
        let added = match accountd.add_account().await {
            Ok(added) => added,
            Err(LinkError::AlreadyAdded(there)) => there,
            Err(e) => return Err(e),
        };
        let granted = accountd.candidates().await?;
        if granted.iter().any(|c| c.account == added) {
            return Ok(Some(added));
        }
        accountd.request_grant().await.map(|c| Some(c.account))
    });
    match ended {
        Ok(_) => read_from(store, accountd).map(Added::Account),
        // Closing the sheet is an answer, not a failure.
        Err(LinkError::Refused(Refusal::Dismissed)) => Ok(Added::Nothing),
        Err(e) => Err(e.to_string()),
    }
}

/// Ask accountd to let Mail use an account again, after the grant on it was withdrawn: its
/// chooser and consent sheet (Mail draws no dialog of its own). What it grants is read into the
/// store, as an add is; closing the sheet is an answer, not a failure.
pub fn allow(store: &SqliteStore, accountd: &Arc<dyn Accountd>) -> Result<Added, String> {
    use mail_runtime::link::LinkError;
    use porter_core::wire::Refusal;
    match with_runtime(accountd.request_grant()) {
        Ok(_) => read_from(store, accountd).map(Added::Account),
        Err(LinkError::Refused(Refusal::Dismissed)) => Ok(Added::Nothing),
        Err(e) => Err(e.to_string()),
    }
}

/// What the account list says when accountd is linked and some accounts here were signed in by
/// Mail itself: they are not loaded, synced or offered while linked, and the way to use them is
/// to add them again.
pub const HELD_LINE: &str =
    "Some accounts were signed in by Mail itself. Add them again to use them here.";

/// Sets aside the accounts Mail signed in itself when `link` is accountd's, and takes them back
/// when it is not. Nothing is written or deleted: their plans, their mail and their keyring
/// items stay as they are for a start that is not linked. Called once, as soon as the link is
/// chosen and before anything lists or syncs.
pub fn hold_back(store: &SqliteStore, link: &Link) {
    store.set_granted_only(link.accountd().is_some());
}

/// [`HELD_LINE`], when the store is set aside from accounts Mail signed in itself and has some.
pub fn held_line(store: &SqliteStore) -> Option<&'static str> {
    (store.granted_only() && !store.held_accounts().is_empty()).then_some(HELD_LINE)
}

/// The accounts Mail signed in itself that are set aside while linked, by address, oldest first:
/// what the account list offers to remove.
pub fn held(store: &SqliteStore) -> Vec<(porter_core::AccountId, String)> {
    if !store.granted_only() {
        return Vec::new();
    }
    store
        .held_accounts()
        .into_iter()
        .filter_map(|id| {
            let address = store
                .connection()
                .query_row(
                    "SELECT address FROM accounts WHERE id = ?1",
                    [id.to_string()],
                    |r| r.get::<_, String>(0),
                )
                .ok()?;
            Some((id, address))
        })
        .collect()
}

/// The accounts that are accountd's, for a caller that wants to say so.
pub fn linked(store: &SqliteStore) -> Vec<Linked> {
    account::linked_accounts(store)
}

#[cfg(test)]
#[path = "accountd_tests.rs"]
mod tests;
