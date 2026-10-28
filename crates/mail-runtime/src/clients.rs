//! Which OAuth client this build signs in as: porter's registry, with mailo's earlier file read
//! as a fallback.
//!
//! A client id is deployment configuration, not a property of an account (`AuthPlan::OAuth`
//! carries none), so it is looked up by issuer. The lookup is [`porter_oauth::ClientRegistry`]:
//! the clients porter ships (`/usr/share/porter/clients.toml`) with the person's own laid over
//! them (`$XDG_CONFIG_HOME/porter/clients.toml`). **mailo never writes either file**: the
//! person's own is written by porter's Settings alone.
//!
//! mailo's own earlier registry, `oauth.json` (`signin::ClientRegistry`), is still read for each
//! issuer the porter files do not cover, so a client id registered through mailo keeps working.
//! It is a fallback to be dropped once porter has imported it.
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
const SHIPPED: Option<&str> = if cfg!(any(target_os = "macos", windows)) {
    None
} else {
    Some("/usr/share/porter/clients.toml")
};

/// The clients file this repository ships (`packaging/oauth/clients.toml`), on every OS so a test
/// can read it; only macOS and Windows build it into the program, where there is no
/// `/usr/share/porter` to read.
const PACKAGED: &str = include_str!("../../../packaging/oauth/clients.toml");

/// The clients built into this program: [`PACKAGED`] where there is no porter install to read.
const BUILT_IN: Option<&str> = if cfg!(any(target_os = "macos", windows)) {
    Some(PACKAGED)
} else {
    None
};

/// The start of a client id that stands in for one not yet registered.
const PLACEHOLDER: &str = "REPLACE-ME";

/// Whether `entry` is a placeholder from the packaged file, not a registered client. `load`
/// drops such a row, so it is never used to sign in: it is read as no client at all.
fn is_placeholder(entry: &ClientEntry) -> bool {
    entry.client_id.0.starts_with(PLACEHOLDER)
}

/// The files a registry is read from, and the clients built into the program.
#[derive(Debug, Clone, Default)]
pub struct Files {
    /// The clients built into the program (a `clients.toml`'s text), under the shipped file.
    pub built_in: Option<&'static str>,
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
            built_in: BUILT_IN,
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

/// The registry from `files`: the built-in clients, porter's over them, then mailo's earlier file
/// for each issuer they do not cover. A file that does not exist is empty; one that is damaged
/// is an error, never read as empty (that would report a configured client as missing). Nothing
/// is written.
pub fn load(files: &Files) -> Result<ClientRegistry, RuntimeError> {
    let mut shipped = read(files.shipped.as_deref())?;
    if let Some(text) = files.built_in {
        let built_in = parse_clients(text).map_err(|e| {
            RuntimeError::Secrets(Failure::new("the built-in clients are unreadable", e))
        })?;
        for entry in built_in.clients {
            if !shipped
                .clients
                .iter()
                .any(|c| c.issuer == entry.issuer && c.channel == entry.channel)
            {
                shipped.clients.push(entry);
            }
        }
    }
    let mut own = read(files.own.as_deref())?;
    // A placeholder is dropped before the earlier file is consulted, so it neither signs in nor
    // hides a client registered through mailo's `oauth.json`.
    shipped.clients.retain(|c| !is_placeholder(c));
    own.clients.retain(|c| !is_placeholder(c));
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

/// Whether this installation can sign a personal Microsoft account (Outlook.com, Hotmail) in:
/// it has a Microsoft client, and that client signs in on an endpoint that takes personal
/// accounts (`common` or `consumers`) rather than mailo's default of `organizations`. With none,
/// or with placeholders only, it cannot, and a personal address is told why.
pub fn personal_microsoft_ready(registry: &ClientRegistry) -> bool {
    client(registry, Issuer::Microsoft)
        // A client whose endpoints cannot be worked out signs nobody in, so it takes no personal
        // account either.
        .and_then(|found| endpoints(found).ok())
        .is_some_and(|ends| {
            ["/common/", "/consumers/"]
                .iter()
                .any(|tenant| ends.authorize.as_str().contains(tenant))
        })
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
/// `common`. `common` also accepts personal Outlook.com accounts, which recently created
/// mailboxes are reported to refuse SMTP client authentication for, so a client signs personal
/// accounts in only when its own endpoints say so ([`personal_microsoft_ready`]). An error for an
/// issuer mailo has no mail sign-in for.
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
    // Owner-only, as it always was: the application secret in here is not the user's credential,
    // but a config file nobody else can read costs nothing and removes the question. Written
    // through a rename, so a crash cannot leave the registry half-written and every client in it
    // lost.
    mail_store::write_atomic(path, body.as_bytes(), Some(0o600))
        .map_err(|e| RuntimeError::Io(Failure::new(path.display().to_string(), e)))?;
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
            built_in: None,
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
            built_in: None,
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
    fn a_placeholder_client_is_no_client_and_hides_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // The file this repository ships has placeholders only: no issuer has a client.
        let registry = load(&Files {
            built_in: Some(PACKAGED),
            ..Files::default()
        })
        .unwrap();
        for issuer in [Issuer::Google, Issuer::Microsoft] {
            assert_eq!(id(&registry, issuer), None);
        }
        assert!(!personal_microsoft_ready(&registry));
        // And it does not cover for the client mailo's earlier file has for that issuer.
        let registry = load(&Files {
            built_in: Some(PACKAGED),
            legacy: Some(write(dir.path(), "oauth.json", LEGACY)),
            ..Files::default()
        })
        .unwrap();
        assert_eq!(id(&registry, Issuer::Google).as_deref(), Some("legacy-g"));
    }

    #[test]
    fn a_built_in_client_is_used_once_its_id_is_real_and_the_shipped_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let real = PACKAGED.replace("REPLACE-ME-microsoft-application-client-id", "real-ms");
        let leaked: &'static str = Box::leak(real.into_boxed_str());
        let registry = load(&Files {
            built_in: Some(leaked),
            ..Files::default()
        })
        .unwrap();
        assert_eq!(id(&registry, Issuer::Microsoft).as_deref(), Some("real-ms"));
        assert!(personal_microsoft_ready(&registry));
        let registry = load(&Files {
            built_in: Some(leaked),
            shipped: Some(write(dir.path(), "shipped.toml", SHIPPED_TOML)),
            ..Files::default()
        })
        .unwrap();
        assert_eq!(
            id(&registry, Issuer::Microsoft).as_deref(),
            Some("shipped-ms")
        );
        // A client on the default `organizations` endpoint does not take personal accounts.
        assert!(!personal_microsoft_ready(&registry));
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
