//! The D-Bus link over a bus of its own: a private `dbus-daemon`, a stand-in for accountd that
//! owns `org.quire.Accounts1` and sends the signals mailo follows, and the link on another
//! connection. Nothing here reaches the person's session bus, keyring or accounts: the daemon
//! listens on a socket in a scratch directory, with no service files of its own to start.
#![cfg(all(feature = "quire-desktop", target_os = "linux"))]

use mail_runtime::link::{Change, Here, Link, dbus, start_with};
use porter_core::consent::Usage;
use porter_dbus::{ACCOUNTS_BUS, ACCOUNTS_PATH, ManagerSkeleton};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use zbus::zvariant::ObjectPath;

/// A `dbus-daemon` of our own, killed (by its pid) when this drops.
struct Bus {
    child: Child,
    address: String,
    _dir: tempfile::TempDir,
}

impl Bus {
    fn start() -> Bus {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("services")).expect("services dir");
        let socket = dir.path().join("bus");
        let config = dir.path().join("bus.conf");
        // Only its own (empty) service directory: no installed accountd can be started by it.
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><listen>unix:path={}</listen>\
                 <servicedir>{}</servicedir><auth>EXTERNAL</auth>\
                 <policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>\
                 <allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>",
                socket.display(),
                dir.path().join("services").display()
            ),
        )
        .expect("config");
        let child = Command::new("dbus-daemon")
            .env_clear()
            .env("HOME", dir.path())
            .env("XDG_RUNTIME_DIR", dir.path())
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
            std::thread::sleep(Duration::from_millis(25));
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accountds_signals_reach_mailo_as_changes() {
    let bus = Bus::start();

    // accountd, as far as the signals go.
    let daemon = bus.connect().await;
    daemon
        .object_server()
        .at(ACCOUNTS_PATH, ManagerSkeleton)
        .await
        .expect("the manager");
    daemon.request_name(ACCOUNTS_BUS).await.expect("the name");

    // mailo's link, on its own connection to the same bus.
    let link = dbus::over(bus.connect().await, Usage::Interactive);
    let mut feed = link
        .changes()
        .await
        .expect("the bus answers")
        .expect("a bus link has a feed");

    // The signals as accountd sends them (the skeleton's own emitters are private to its crate).
    let account = ObjectPath::try_from("/org/quire/Accounts1/account/fastmail_me").unwrap();
    let manager = "org.quire.Accounts1.Manager";
    daemon
        .emit_signal(
            None::<&str>,
            ACCOUNTS_PATH,
            manager,
            "AccountAdded",
            &(account.clone(),),
        )
        .await
        .expect("AccountAdded");
    daemon
        .emit_signal(
            None::<&str>,
            ACCOUNTS_PATH,
            manager,
            "AccountRemoved",
            &(account,),
        )
        .await
        .expect("AccountRemoved");
    daemon
        .emit_signal(
            None::<&str>,
            ACCOUNTS_PATH,
            manager,
            "GrantChanged",
            &("grant-1",),
        )
        .await
        .expect("GrantChanged");

    let mut heard = Vec::new();
    for _ in 0..3 {
        let next = tokio::time::timeout(Duration::from_secs(10), feed.next())
            .await
            .expect("a change within ten seconds")
            .expect("the feed is still open");
        heard.push(next);
    }
    // The three streams are merged, so only the set is promised, not the order.
    assert!(heard.contains(&Change::Added), "{heard:?}");
    assert!(heard.contains(&Change::Granted), "{heard:?}");
    // The removal names the account by its object path's last segment: all the signal carries.
    assert!(
        heard.contains(&Change::Removed("fastmail_me".to_owned())),
        "{heard:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_over_a_bus_is_the_one_start_chooses_when_accountd_is_here() {
    let bus = Bus::start();
    let connection = bus.connect().await;
    let chosen = start_with(Here::Yes, async {
        Some(dbus::over(connection, Usage::Background))
    })
    .await;
    assert!(matches!(chosen, Link::Accountd(_)), "{chosen:?}");
}
