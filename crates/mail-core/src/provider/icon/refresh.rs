//! Fetch, decode and store: on `mailo account add`, on an explicit refresh, and once when a window
//! opens for the known providers not yet cached ([`missing`]).

use super::IconError;
use super::cache::{cached, file_stem, store};
use super::decode::decode;
use super::fetch::{client, fetch, url};
use crate::environment::Program;
use crate::error::CoreError;
use crate::provider::Provider;
use std::fmt::Write as _;
use std::path::Path;

/// Re-fetch every provider. One failure does not stop the others.
pub async fn refresh(
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
pub fn report(results: &[(Provider, Result<usize, IconError>)]) -> String {
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
pub fn providers_of(store: &mail_store::SqliteStore) -> Result<Vec<Provider>, CoreError> {
    let mut out = Vec::new();
    for account in store.list_accounts()? {
        let Ok(plan) = account.plan else {
            log::warn!("provider icon: an account plan could not be read");
            continue;
        };
        let which = crate::provider::provider(&plan);
        if !out.contains(&which) {
            out.push(which);
        }
    }
    Ok(out)
}

/// The known providers whose icon is not cached yet, when the installed program is the one running.
///
/// Every one, not only the configured accounts': Add Account lists them all before any account
/// is on one, and an account added in the window would otherwise show its letter until a manual
/// refresh. Empty under a test binary, which must not open a socket or write `~/.cache/mailo`.
pub fn missing(dir: &Path, program: Program) -> Vec<Provider> {
    if program != Program::Installed {
        return Vec::new();
    }
    Provider::ALL
        .into_iter()
        .filter(|provider| url(*provider).is_some() && cached(dir, *provider).is_none())
        .collect()
}

/// Fetch and cache `provider` when the installed program is the one running and the
/// file is not already there.
///
/// Tests execute from `deps/<crate>-<hash>` and must not open a socket or write
/// `~/.cache/mailo`. Awaited, because `mailo account add` returns immediately afterwards, and a
/// process exit kills a fetch nobody waits for. A failure is logged and is not an error for the
/// caller.
pub async fn fetch_if_missing(provider: Provider, program: Program) {
    if url(provider).is_none() || program != Program::Installed {
        return;
    }
    let Some(root) = crate::config::cache_dir() else {
        return;
    };
    let dir = root.join("providers");
    if cached(&dir, provider).is_some() {
        return;
    }
    match fetch_one(&dir, provider).await {
        Ok(()) => {}
        Err(err) => log::warn!("provider icon: {provider:?}: {err}"),
    }
}

async fn fetch_one(dir: &Path, provider: Provider) -> Result<(), IconError> {
    let http = client()?;
    let bytes = fetch(&http, provider).await?;
    let png = decode(&bytes)?;
    store(dir, provider, &png)
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
