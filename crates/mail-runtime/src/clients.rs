//! Which OAuth client this build signs in as: porter's registry, with mailo's earlier file read
//! as a fallback.
//!
//! A client id is deployment configuration, not a property of an account (`AuthPlan::OAuth`
//! carries none), so it is looked up by issuer. The lookup is [`porter_oauth::ClientRegistry`]:
//! the clients porter ships (`/usr/share/porter/clients.toml`) with the person's own laid over
//! them (`$XDG_CONFIG_HOME/porter/clients.toml`). **mailo never writes either file**: the
//! person's own is written by porter's Settings alone.
//!
//! Before E4 mailo kept its own registry in `oauth.json` (`signin::ClientRegistry`). That file is
//! still read, for each issuer the porter files do not cover, so a client id registered through
//! mailo keeps working. It is a fallback to be dropped once porter has imported it (FINDINGS F202).
//!
//! [`remember`] is the one write left, and it is to mailo's own `oauth.json`, as before: an
//! account signed in with `MAILO_OAUTH_CLIENT_ID` could not be renewed an hour later otherwise.

use crate::RuntimeError;
use crate::error::Failure;
use porter_core::{EndpointUrl, SecretText};
use porter_oauth::ClientRegistry;
use porter_provider::{
    ClientChannel, ClientEntry, ClientId, ClientsFile, Issuer, IssuerEndpoints, parse_clients,
};
use std::path::{Path, PathBuf};

/// The channel mailo's ids are registered for. mailo has no build channels of its own.
pub const CHANNEL: ClientChannel = ClientChannel::Stable;

/// Where porter ships its clients, on a system that has porter installed.
#[cfg(not(any(target_os = "macos", windows)))]
const SHIPPED: Option<&str> = Some("/usr/share/porter/clients.toml");
#[cfg(any(target_os = "macos", windows))]
const SHIPPED: Option<&str> = None;

/// The three files a registry is read from.
#[derive(Debug, Clone, Default)]
pub struct Files {
    /// porter's shipped clients.
    pub shipped: Option<PathBuf>,
    /// The person's own clients (porter's file; read only).
    pub own: Option<PathBuf>,
    /// mailo's earlier `oauth.json`.
    pub legacy: Option<PathBuf>,
}

impl Files {
    /// Where this installation keeps them.
    pub fn here() -> Self {
        Self {
            shipped: SHIPPED.map(PathBuf::from),
            own: crate::places::porter_config().map(|dir| dir.join("clients.toml")),
            legacy: legacy_path(),
        }
    }
}

/// `oauth.json` in mailo's config directory.
pub fn legacy_path() -> Option<PathBuf> {
    Some(crate::places::dir(crate::places::Place::Config)?.join("oauth.json"))
}

/// The registry of this installation.
pub fn load_default() -> Result<ClientRegistry, RuntimeError> {
    load(&Files::here())
}

/// The registry from `files`: porter's first, then mailo's earlier file for each issuer they do
/// not cover. A file that does not exist is empty; one that is damaged is an error, never read
/// as empty (that would report a configured client as missing). Nothing is written.
pub fn load(files: &Files) -> Result<ClientRegistry, RuntimeError> {
    let mut shipped = read(files.shipped.as_deref())?;
    let own = read(files.own.as_deref())?;
    if let Some(path) = &files.legacy
        && let Some(text) = text(path)?
    {
        let legacy = porter_oauth::from_mailo(&text, CHANNEL).map_err(|e| {
            RuntimeError::Secrets(Failure::new(format!("{}: unreadable", path.display()), e))
        })?;
        for entry in legacy.clients {
            let covered = |file: &ClientsFile| {
                file.clients
                    .iter()
                    .any(|c| c.issuer == entry.issuer && c.channel == entry.channel)
            };
            if !covered(&shipped) && !covered(&own) {
                shipped.clients.push(entry);
            }
        }
    }
    Ok(ClientRegistry::layered(shipped, own))
}

/// A registry of exactly `clients`, for a caller that has them in hand.
pub fn registry_of(clients: Vec<ClientEntry>) -> ClientRegistry {
    ClientRegistry::layered(ClientsFile { clients }, ClientsFile::default())
}

/// The client for `issuer`, if this installation has one.
pub fn client(registry: &ClientRegistry, issuer: Issuer) -> Option<&ClientEntry> {
    registry.lookup(issuer, CHANNEL)
}

/// A client a person typed in (`MAILO_OAUTH_CLIENT_ID`), as a registry entry.
pub fn entry(issuer: Issuer, client_id: &str, client_secret: Option<&str>) -> ClientEntry {
    ClientEntry {
        issuer,
        channel: CHANNEL,
        client_id: ClientId(client_id.to_owned()),
        client_secret: client_secret.map(SecretText::new),
        endpoints: None,
    }
}

/// Where to send `client`'s sign-in and renewals: its own endpoints when it names some, else the
/// issuer's.
///
/// mailo's one departure from the issuer's published ones is Microsoft's: `organizations`, not
/// `common`. `common` also accepts personal Outlook.com accounts, and those are a worse case this
/// does not support: basic authentication was retired there on 2024-09-16 and recently created
/// personal mailboxes are reported to have SMTP client authentication permanently off, so a
/// `common` endpoint would hand back a perfectly good token that then fails at submission with
/// nothing to explain it. Refusing at sign-in, where the user can read the reason, is the better
/// failure. An error for an issuer mailo has no mail sign-in for.
pub fn endpoints(client: &ClientEntry) -> Result<IssuerEndpoints, RuntimeError> {
    if let Some(own) = &client.endpoints {
        return Ok(own.clone());
    }
    match client.issuer {
        Issuer::Google => Ok(Issuer::Google.endpoints()),
        Issuer::Microsoft => {
            let url = |text: &str| {
                EndpointUrl::parse(text)
                    .unwrap_or_else(|_| unreachable!("{text} is a literal endpoint URL"))
            };
            Ok(IssuerEndpoints {
                authorize: url(
                    "https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize",
                ),
                token: url("https://login.microsoftonline.com/organizations/oauth2/v2.0/token"),
                revoke: None,
                device: None,
            })
        }
        other => Err(RuntimeError::Secrets(Failure::said(format!(
            "mailo has no mail sign-in through {other:?}"
        )))),
    }
}

/// Record the client an account signed in with in mailo's own `oauth.json`, so it can be renewed
/// later. Never porter's file. Returns where it was written, or `None` when this machine has no
/// config directory.
pub fn remember(
    legacy: Option<&Path>,
    issuer: Issuer,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<Option<PathBuf>, RuntimeError> {
    let Some(path) = legacy else {
        return Ok(None);
    };
    // Re-read rather than overwritten: a second account with another issuer must not erase the
    // first one's row. Rows this function does not know stay as they were.
    let mut rows: Vec<serde_json::Value> = match text(path)? {
        None => Vec::new(),
        Some(text) => {
            let file: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                RuntimeError::Secrets(Failure::new(format!("{}: unreadable", path.display()), e))
            })?;
            file.get("registrations")
                .and_then(|r| r.as_array().cloned())
                .unwrap_or_default()
        }
    };
    let issuer_name = serde_json::to_value(issuer)
        .map_err(|e| RuntimeError::Secrets(Failure::new("cannot encode the registry", e)))?;
    let mut row = serde_json::json!({ "issuer": issuer_name, "client_id": client_id });
    if let Some(secret) = client_secret {
        row["client_secret"] = secret.into();
    }
    match rows
        .iter_mut()
        .find(|r| r.get("issuer") == Some(&issuer_name))
    {
        Some(existing) => *existing = row,
        None => rows.push(row),
    }
    let body = serde_json::to_string_pretty(&serde_json::json!({ "registrations": rows }))
        .map_err(|e| RuntimeError::Secrets(Failure::new("cannot encode the registry", e)))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RuntimeError::Io(Failure::new(parent.display().to_string(), e)))?;
    }
    std::fs::write(path, body)
        .map_err(|e| RuntimeError::Io(Failure::new(path.display().to_string(), e)))?;
    // Owner-only, as it always was: the application secret in here is not the user's credential,
    // but a config file nobody else can read costs one syscall and removes the question.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(Some(path.to_owned()))
}

fn text(path: &Path) -> Result<Option<String>, RuntimeError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(RuntimeError::Io(Failure::new(
            path.display().to_string(),
            e,
        ))),
    }
}

fn read(path: Option<&Path>) -> Result<ClientsFile, RuntimeError> {
    let Some(path) = path else {
        return Ok(ClientsFile::default());
    };
    match text(path)? {
        None => Ok(ClientsFile::default()),
        Some(text) => parse_clients(&text).map_err(|e| {
            RuntimeError::Secrets(Failure::new(format!("{}: unreadable", path.display()), e))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(registry: &ClientRegistry, issuer: Issuer) -> Option<String> {
        client(registry, issuer).map(|c| c.client_id.0.clone())
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    const SHIPPED_TOML: &str =
        "[[client]]\nissuer = \"microsoft\"\nchannel = \"stable\"\nclient_id = \"shipped-ms\"\n";
    const OWN_TOML: &str =
        "[[client]]\nissuer = \"google\"\nchannel = \"stable\"\nclient_id = \"own-g\"\n";
    const LEGACY: &str = r#"{"registrations":[
        {"issuer":"google","client_id":"legacy-g","client_secret":"GOCSPX-x"},
        {"issuer":"microsoft","client_id":"legacy-ms"},
        {"issuer":"dropbox","client_id":"legacy-db"}]}"#;

    #[test]
    fn porters_registry_wins_and_the_legacy_file_fills_the_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let files = Files {
            shipped: Some(write(dir.path(), "shipped.toml", SHIPPED_TOML)),
            own: Some(write(dir.path(), "own.toml", OWN_TOML)),
            legacy: Some(write(dir.path(), "oauth.json", LEGACY)),
        };
        let before: Vec<_> = ["shipped.toml", "own.toml", "oauth.json"]
            .map(|n| std::fs::read(dir.path().join(n)).unwrap())
            .into();
        let registry = load(&files).unwrap();
        // The shipped Microsoft client and the person's own Google client beat mailo's.
        assert_eq!(
            id(&registry, Issuer::Microsoft).as_deref(),
            Some("shipped-ms")
        );
        assert_eq!(id(&registry, Issuer::Google).as_deref(), Some("own-g"));
        // An issuer porter has no client for is mailo's.
        assert_eq!(id(&registry, Issuer::Dropbox).as_deref(), Some("legacy-db"));
        assert_eq!(id(&registry, Issuer::Box), None);
        // Nothing was written: no file changed and none appeared.
        let after: Vec<_> = ["shipped.toml", "own.toml", "oauth.json"]
            .map(|n| std::fs::read(dir.path().join(n)).unwrap())
            .into();
        assert_eq!(before, after);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }

    #[test]
    fn a_legacy_client_keeps_its_secret_and_endpoints_when_porter_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = r#"{"registrations":[
            {"issuer":"google","client_id":"g","client_secret":"GOCSPX-x"},
            {"issuer":"microsoft","client_id":"m","endpoints":
              {"auth":"https://login.microsoftonline.us/a","token":"https://login.microsoftonline.us/t"}}]}"#;
        let registry = load(&Files {
            legacy: Some(write(dir.path(), "oauth.json", legacy)),
            ..Files::default()
        })
        .unwrap();
        let google = client(&registry, Issuer::Google).unwrap();
        assert_eq!(google.client_secret.as_ref().unwrap().expose(), "GOCSPX-x");
        let microsoft = client(&registry, Issuer::Microsoft).unwrap();
        assert_eq!(
            endpoints(microsoft).unwrap().token.as_str(),
            "https://login.microsoftonline.us/t"
        );
    }

    #[test]
    fn missing_files_are_an_empty_registry_and_a_damaged_one_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let absent = |n: &str| Some(dir.path().join(n));
        let registry = load(&Files {
            shipped: absent("a"),
            own: absent("b"),
            legacy: absent("c"),
        })
        .unwrap();
        assert_eq!(id(&registry, Issuer::Google), None);
        for damaged in [
            Files {
                legacy: Some(write(dir.path(), "oauth.json", "{ not json")),
                ..Files::default()
            },
            Files {
                own: Some(write(dir.path(), "own.toml", "[[client")),
                ..Files::default()
            },
        ] {
            let error = load(&damaged).unwrap_err().to_string();
            assert!(error.contains("unreadable"), "{error}");
        }
    }

    #[test]
    fn microsofts_mail_sign_in_is_the_organizations_tenant_and_other_issuers_are_refused() {
        let ms = endpoints(&entry(Issuer::Microsoft, "x", None)).unwrap();
        assert!(ms.authorize.as_str().contains("/organizations/"), "{ms:?}");
        assert!(ms.token.as_str().contains("/organizations/"), "{ms:?}");
        assert_eq!(
            endpoints(&entry(Issuer::Google, "x", None)).unwrap(),
            Issuer::Google.endpoints()
        );
        assert!(endpoints(&entry(Issuer::Dropbox, "x", None)).is_err());
    }

    #[test]
    fn remember_writes_mailos_own_file_only_and_replaces_per_issuer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mailo").join("oauth.json");
        remember(Some(&path), Issuer::Google, "first", Some("GOCSPX-x")).unwrap();
        remember(Some(&path), Issuer::Microsoft, "ms", None).unwrap();
        remember(Some(&path), Issuer::Google, "second", None).unwrap();
        let registry = load(&Files {
            legacy: Some(path.clone()),
            ..Files::default()
        })
        .unwrap();
        assert_eq!(id(&registry, Issuer::Google).as_deref(), Some("second"));
        assert_eq!(id(&registry, Issuer::Microsoft).as_deref(), Some("ms"));
        assert!(
            client(&registry, Issuer::Google)
                .unwrap()
                .client_secret
                .is_none()
        );
        let text = std::fs::read_to_string(&path).unwrap();
        for forbidden in ["access_token", "refresh_token"] {
            assert!(!text.contains(forbidden), "{text}");
        }
        assert_eq!(remember(None, Issuer::Google, "x", None).unwrap(), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "mode is {mode:o}");
        }
    }
}
