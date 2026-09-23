//! Fetch, decode and store: on `mailo account add`, and on an explicit refresh.

use super::IconError;
use super::cache::{cached, file_stem, store};
use super::decode::decode;
use super::fetch::{client, fetch, url};
use crate::provider::Provider;
use std::fmt::Write as _;
use std::path::Path;

/// Re-fetch every provider. One failure does not stop the others.
pub(crate) async fn refresh(
    dir: &Path,
    providers: &[Provider],
) -> Vec<(Provider, Result<usize, IconError>)> {
    let client = match client() {
        Ok(client) => client,
        Err(err) => {
            let message = err.to_string();
            return providers
                .iter()
                .copied()
                .map(|provider| (provider, Err(IconError::Fetch(message.clone()))))
                .collect();
        }
    };
    let mut out = Vec::with_capacity(providers.len());
    for provider in providers.iter().copied() {
        let result = fetch_into(&client, dir, provider).await;
        out.push((provider, result));
    }
    out
}

/// What `mailo icons refresh` prints. Failures are a line here; the caller logs them.
pub(crate) fn report(results: &[(Provider, Result<usize, IconError>)]) -> String {
    let mut out = String::new();
    for (provider, result) in results {
        let stem = file_stem(*provider);
        match result {
            Ok(bytes) => {
                let _ = writeln!(out, "{stem}: wrote {bytes} bytes");
            }
            Err(IconError::Unmapped) => {
                let _ = writeln!(out, "{stem}: letters only");
            }
            Err(_) => {
                let _ = writeln!(out, "{stem}: not updated");
            }
        }
    }
    out
}

/// The providers of the configured accounts, in the order the accounts were added.
///
/// A plan that does not parse is logged and skipped. The same provider twice is
/// one fetch.
pub(crate) fn providers_of(store: &mail_store::SqliteStore) -> Result<Vec<Provider>, String> {
    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT plan FROM accounts ORDER BY created_at")
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| err.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let text = row.map_err(|err| err.to_string())?;
        let Ok(plan) = serde_json::from_str::<mail_domain::AccountPlan>(&text) else {
            eprintln!("provider icon: an account plan could not be read");
            continue;
        };
        let which = crate::provider::provider(&plan);
        if !out.contains(&which) {
            out.push(which);
        }
    }
    Ok(out)
}

/// Fetch and cache `provider` when the user binary is the one running and the
/// file is not already there.
///
/// Tests execute from `deps/<crate>-<hash>` and must not open a socket or write
/// `~/.cache/mailo`. The thread is joined because `mailo account add` returns
/// immediately afterwards, and a process exit kills a thread it does not wait
/// for. A failure is logged and is not an error for the caller.
pub(crate) fn fetch_if_missing(provider: Provider) {
    if url(provider).is_none() || !user_binary() {
        return;
    }
    let Some(root) = crate::appearance::cache_dir() else {
        return;
    };
    let dir = root.join("providers");
    if cached(&dir, provider).is_some() {
        return;
    }
    let handle = std::thread::spawn(move || fetch_one(&dir, provider));
    match handle.join() {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("provider icon: {provider:?}: {err}"),
        Err(_) => eprintln!("provider icon: {provider:?}: the fetch stopped"),
    }
}

pub(super) fn user_binary() -> bool {
    let Ok(path) = std::env::current_exe() else {
        return false;
    };
    if path
        .parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == "deps")
    {
        return false;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "mailo")
}

fn fetch_one(dir: &Path, provider: Provider) -> Result<(), IconError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| IconError::Fetch(err.to_string()))?;
    runtime.block_on(async {
        let http = client()?;
        let bytes = fetch(&http, provider).await?;
        let png = decode(&bytes)?;
        store(dir, provider, &png)
    })
}

async fn fetch_into(
    client: &reqwest::Client,
    dir: &Path,
    provider: Provider,
) -> Result<usize, IconError> {
    if url(provider).is_none() {
        return Err(IconError::Unmapped);
    }
    let bytes = fetch(client, provider).await?;
    let png = decode(&bytes)?;
    let len = png.len();
    store(dir, provider, &png)?;
    Ok(len)
}
