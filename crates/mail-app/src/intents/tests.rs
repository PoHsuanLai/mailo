//! The provider over a real bus: a private `dbus-daemon`, the router's name taken by the test,
//! and mailo answering on its own. Nothing here reaches the person's session bus, keyring or
//! mail: the daemon listens on a socket in a scratch directory and the store is in memory.

use super::*;
use crate::intents::provider::tests::world;
use crate::intents::wire::{Invocation, answer_of};
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use zbus::zvariant::OwnedValue;

/// A `dbus-daemon` of our own, killed when this drops.
struct Bus {
    child: Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl Bus {
    fn start() -> Bus {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("bus");
        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
                socket.display()
            ),
        )
        .expect("config");
        let child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .arg("--nofork")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("these tests need dbus-daemon on PATH, to run a bus of their own");
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(socket.exists(), "the private bus did not come up");
        Bus {
            child,
            address: format!("unix:path={}", socket.display()),
            _dir: dir,
        }
    }

    async fn connect(&self) -> zbus::Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .expect("an address")
            .build()
            .await
            .expect("a connection")
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Async, and awaited inside the test's `zbus::block_on`: with zbus's `tokio` feature on (oo7
/// turns it on for account secrets, E2) `zbus::block_on` is a tokio runtime's, and one inside
/// another panics.
async fn proxy<'a>(connection: &'a zbus::Connection) -> zbus::Proxy<'a> {
    zbus::Proxy::new(connection, APP, PATH, "org.quire.IntentProvider1")
        .await
        .expect("a proxy")
}

type Options = HashMap<String, OwnedValue>;

async fn ask(proxy: &zbus::Proxy<'_>, member: &str, argument: &str) -> zbus::Result<String> {
    match member {
        "Perform" | "DryRun" => proxy.call(member, &(argument, Options::new())).await,
        "Search" => proxy.call(member, &(argument, 0u64)).await,
        "Undo" => proxy.call(member, &(argument, "{\"kind\":\"cli\"}")).await,
        _ => proxy.call(member, &(argument,)).await,
    }
}

/// The router's label for what the person typed.
fn user_label() -> serde_json::Value {
    serde_json::json!({ "integrity": "trusted", "confidentiality": { "kind": "public" },
                        "classes": [], "sources": [{ "kind": "user" }] })
}

fn invocation(action: &str, keys: &[String], args: serde_json::Value) -> String {
    serde_json::json!({
        "call": 7,
        "action": action,
        "target": { "kind": "entities", "v": keys.iter().map(|key| serde_json::json!({
            "app": APP, "kind": "mail.thread", "key": key })).collect::<Vec<_>>() },
        "args": args,
        "actor": { "kind": "cli" },
        "origin": "cli",
        "space": "work",
    })
    .to_string()
}

#[test]
fn the_router_reaches_mailo_over_a_bus_and_nobody_else_does() {
    let bus = Bus::start();
    zbus::block_on(async {
        let (provider, store, _dir) = world();
        let provider_connection = bus.connect().await;
        serve_on(&provider_connection, provider)
            .await
            .expect("serving");

        // Another process answering for the name is refused, not queued behind.
        let second = bus.connect().await;
        let (again, _, _again_dir) = world();
        assert!(matches!(
            serve_on(&second, again).await,
            Err(ServeError::Taken)
        ));

        // The router: the owner of the router's name.
        let router = bus.connect().await;
        router
            .request_name(ROUTER)
            .await
            .expect("the router's name");
        let calls = proxy(&router).await;
        let thread = mail_domain::ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
        let key = thread.to_string();

        let answered = ask(
            &calls,
            "Perform",
            &invocation(
                "mail.thread.archive",
                std::slice::from_ref(&key),
                serde_json::json!({}),
            ),
        )
        .await
        .expect("performed");
        let answered: serde_json::Value = serde_json::from_str(&answered).expect("json");
        assert_eq!(answered["Ok"]["said"], "Archived", "{answered}");
        assert_eq!(answered["Ok"]["undo"]["kind"], "yes");
        assert_eq!(
            answered["Ok"]["follow"],
            serde_json::json!({ "kind": "nothing" })
        );
        use mail_store::Store as _;
        let in_inbox = |store: &mail_store::SqliteStore| {
            store
                .thread(thread)
                .expect("thread")
                .summary
                .mailboxes
                .contains(mail_domain::MailboxRole::Inbox)
        };
        assert!(!in_inbox(&store), "archive over the bus changed nothing");

        // The token comes back as the router holds it: a JSON string.
        let token = serde_json::to_string(&answered["Ok"]["undo"]["v"]).expect("token");
        let undone = ask(&calls, "Undo", &token).await.expect("undone");
        assert_eq!(undone, r#"{"Ok":null}"#);
        assert!(in_inbox(&store));
        let again = ask(&calls, "Undo", &token).await.expect("answered");
        assert_eq!(again, r#"{"Err":"gone"}"#);

        // A refusal is the router's `Err`.
        let missing = ask(
            &calls,
            "Perform",
            &invocation(
                "mail.thread.nope",
                std::slice::from_ref(&key),
                serde_json::json!({}),
            ),
        )
        .await
        .expect("answered");
        assert_eq!(missing, r#"{"Err":{"kind":"unsupported"}}"#);

        let found = ask(&calls, "Search", "lunch").await.expect("searched");
        let found: serde_json::Value = serde_json::from_str(&found).expect("json");
        assert_eq!(found.as_array().map(Vec::len), Some(2), "{found}");

        let preview = ask(&calls, "DryRun", &invocation(
            "mail.message.send", &[],
            serde_json::json!({ "to": { "value": { "kind": "text", "v": "ada@b.c" }, "label": user_label() },
                                "body": { "value": { "kind": "text", "v": "Hi" }, "label": user_label() } }),
        )).await.expect("a preview");
        assert!(
            preview.contains(r#""kind":"message""#) && preview.contains("ada@b.c"),
            "{preview}"
        );

        let context = ask(&calls, "Context", r#"{"kind":"active_window"}"#)
            .await
            .expect("context");
        assert!(context.contains(r#""kind":"nowhere""#), "{context}");

        // Neither Resolve nor a malformed argument is an answer.
        assert!(ask(&calls, "Resolve", "[]").await.is_err());
        assert!(ask(&calls, "Perform", "not json").await.is_err());

        // A stranger is refused every member but Summon, and changes nothing.
        let stranger = bus.connect().await;
        let theirs = proxy(&stranger).await;
        let refused = ask(
            &theirs,
            "Perform",
            &invocation(
                "mail.thread.archive",
                std::slice::from_ref(&key),
                serde_json::json!({}),
            ),
        )
        .await;
        assert!(
            matches!(&refused, Err(zbus::Error::MethodError(name, _, _)) if name.as_str() == "org.freedesktop.DBus.Error.AccessDenied"),
            "{refused:?}"
        );
        assert!(in_inbox(&store), "a stranger archived a conversation");
        for member in ["Search", "Preview", "Context", "Suggest"] {
            assert!(ask(&theirs, member, "{}").await.is_err(), "{member}");
        }
        let summoned: String = theirs
            .call("Summon", &(1u64, r#"{"kind":"double_tap"}"#))
            .await
            .expect("summon");
        assert_eq!(summoned, "\"declined\"");
    });
}

#[test]
fn the_interface_is_the_one_the_router_declares() {
    use zbus::object_server::Interface;
    let (provider, _store, _dir) = world();
    let object = serve::Object { provider };
    let mut served = String::from(
        "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n<node>\n",
    );
    object.introspect_to_writer(&mut served, 1);
    served.push_str("</node>\n");
    assert_eq!(served, include_str!("IntentProvider1.xml"));
}

/// The facts of `docket-core`'s manifest rules that a manifest of ours can break, read from the
/// file as installed. The router's own validator is the authority (`docket-eval --check-app`
/// runs it over this repository); this holds the file to the code in a build that has no router.
#[test]
fn the_manifest_declares_what_the_provider_answers_and_obeys_the_routers_rules() {
    let manifest: toml::Table = MANIFEST.parse().expect("the manifest is TOML");
    assert_eq!(manifest["app"].as_str(), Some(APP));
    assert_eq!(manifest["vocab"].as_integer(), Some(1));
    let actions = manifest["actions"].as_array().expect("actions");
    let mut declared: Vec<&str> = actions
        .iter()
        .map(|a| a["name"].as_str().expect("a name"))
        .collect();
    let mut answered = Provider::actions();
    declared.sort_unstable();
    answered.sort_unstable();
    assert_eq!(
        declared, answered,
        "the manifest and the provider disagree on what mailo does"
    );

    let kinds: Vec<&str> = manifest["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .map(|e| e["kind"].as_str().expect("a kind"))
        .collect();
    for action in actions {
        let name = action["name"].as_str().expect("a name");
        assert!(
            name.starts_with("mail."),
            "{name} is outside the app's prefix"
        );
        let (effect, undo) = (
            action["effect"].as_str().expect("effect"),
            action["undo"].as_str().expect("undo"),
        );
        assert_eq!(
            effect == "read",
            undo == "not_undoable",
            "{name}: a read has no undo, a write has a token"
        );
        for key in [
            "label", "on", "classes", "reach", "latency", "result", "keys", "lasting", "dry_run",
            "params",
        ] {
            assert!(action.get(key).is_some(), "{name} does not write {key}");
        }
        let params = action["params"].as_array().expect("params");
        let sinks: Vec<&str> = params
            .iter()
            .map(|p| p["sink"].as_str().expect("sink"))
            .collect();
        if effect == "outbound" {
            assert!(
                sinks
                    .iter()
                    .any(|s| matches!(*s, "recipient" | "destination")),
                "{name} sends and names no recipient"
            );
        }
        assert_eq!(
            action["dry_run"].as_str() == Some("preview"),
            effect == "outbound",
            "{name}: what sends mail has a preview, and nothing else does"
        );
        for kind in [
            action["on"].get("v"),
            action["result"].get("v").filter(|v| v.is_str()),
        ]
        .into_iter()
        .flatten()
        {
            assert!(
                kinds.contains(&kind.as_str().expect("a kind")),
                "{name} names {kind}"
            );
        }
    }
}

#[test]
fn the_answers_are_the_routers_ok_and_err() {
    let ok: Result<u8, u8> = Ok(1);
    let err: Result<u8, u8> = Err(2);
    assert_eq!(
        (answer_of(&ok), answer_of(&err)),
        (r#"{"Ok":1}"#.to_owned(), r#"{"Err":2}"#.to_owned())
    );
    let _ = Invocation::text;
}

/// The package's D-Bus service file is the source install's with the package's path, so that the
/// two cannot name different things.
#[test]
fn the_packaged_service_file_is_the_installed_one_with_the_packages_path() {
    let body = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let installed = include_str!("../../../../dist/org.quire.Mail.service.in")
        .replace("@BIN@", "/usr/bin/mailo");
    let packaged = include_str!("../../../../packaging/org.quire.Mail.service");
    assert_eq!(body(&installed), body(packaged));
    assert!(body(packaged).contains(&format!("Name={APP}")));
}

/// The companion's mail skill names only actions this manifest declares and offers, and keeps
/// docket's skill rules (`docket-eval --check-skills` is the authority; this holds the files to
/// the manifest in a build that has no docket). A renamed or hidden action would hide the skill.
#[test]
fn the_mail_skill_teaches_only_offered_actions_and_keeps_the_skill_rules() {
    let skill: toml::Table = include_str!("../../../../dist/skills/mail/skill.toml")
        .parse()
        .expect("skill.toml is TOML");
    let text = include_str!("../../../../dist/skills/mail/SKILL.md");
    assert_eq!(skill["vocab"].as_integer(), Some(1));
    assert_eq!(skill["id"].as_str(), Some("mail"));
    assert_eq!(skill["owner"].as_str(), Some(APP));

    let manifest: toml::Table = MANIFEST.parse().expect("the manifest is TOML");
    let offered: Vec<&str> = manifest["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|a| a["reach"].as_str() != Some("hidden"))
        .map(|a| a["name"].as_str().expect("a name"))
        .collect();
    let uses = skill["uses"].as_array().expect("uses");
    assert!(!uses.is_empty());
    for used in uses {
        let used = used.as_str().expect("a use");
        let action = used
            .strip_prefix(&format!("{APP}:"))
            .unwrap_or_else(|| panic!("{used} is not one of {APP}'s"));
        assert!(
            offered.contains(&action),
            "{used} is not an action the manifest offers"
        );
    }

    let (front, body) = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .expect("SKILL.md opens with a --- block");
    assert!(front.lines().any(|line| line == "name: mail"), "{front}");
    let description = front
        .lines()
        .find_map(|line| line.strip_prefix("description: "))
        .expect("a description");
    assert!(description.chars().count() <= 160, "{description}");
    assert!(!body.trim().is_empty());
    assert!(body.len() <= 3 * 1024, "the body is {} bytes", body.len());
}
