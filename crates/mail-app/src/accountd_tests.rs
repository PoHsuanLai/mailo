//! What the link does to the store, against a table standing in for accountd; and the capability
//! probe against a bus of our own.

use super::*;
use mail_runtime::Transport;
use mail_runtime::link::{Answer, Changes, LinkError, Listed};
use porter_core::capability::{
    Access, Capability, Delta, LabelModel, MailCap, MailTransport, Offered,
};
use porter_core::wire::Refusal;
use porter_core::{
    AccountId, AccountLabel, Audience, Candidate, EndpointUrl, Family, GrantId, IssuedToken,
    LoginName, ProviderId, Restriction, ServiceEndpoint, Subject,
};
use std::collections::VecDeque;
use std::sync::Mutex;

fn candidate(account: &str, address: &str, grant: &str) -> Candidate {
    let endpoint = |family, url: &str, tls| ServiceEndpoint {
        family,
        url: EndpointUrl::parse(url).unwrap(),
        tls,
        login: LoginName(address.to_owned()),
    };
    Candidate {
        account: AccountId::parse(account).unwrap(),
        label: AccountLabel(address.to_owned()),
        provider: ProviderId::parse("fastmail").unwrap(),
        subject: Subject::Account,
        capability: Capability::Mail(MailCap {
            access: Access::ReadWrite,
            send: Offered::Present,
            delta: Delta::Push,
            transport: MailTransport::Imap,
            labels: LabelModel::Folders,
        }),
        restriction: Restriction::none(),
        grant: GrantId::parse(grant).unwrap(),
        endpoints: vec![
            endpoint(
                Family::Imap,
                "imaps://imap.example.test:993",
                porter_core::Tls::Implicit,
            ),
            endpoint(
                Family::Smtp,
                "smtps://smtp.example.test:465",
                porter_core::Tls::Implicit,
            ),
        ],
    }
}

/// What the add-account sheet answers with: the account it made, and the candidate Mail is offered
/// for it, if the sheet gave Mail a grant.
type Sheet = Result<(AccountId, Option<Candidate>), LinkError>;

/// accountd as a table: what it offers now, what the sheet adds, what was asked of it.
#[derive(Debug, Default)]
struct Daemon {
    offered: Mutex<Vec<Candidate>>,
    /// What the add-account sheet answers with, and what it would then offer.
    adds: Mutex<VecDeque<Sheet>>,
    /// What the consent sheet gives.
    consents: Mutex<VecDeque<Result<Candidate, LinkError>>>,
    asked: Mutex<Vec<String>>,
}

impl Daemon {
    fn offering(candidates: Vec<Candidate>) -> Arc<Daemon> {
        let daemon = Daemon::default();
        *daemon.offered.lock().unwrap() = candidates;
        Arc::new(daemon)
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

fn unsaid<T>() -> Answer<'static, T> {
    Box::pin(async { Err(LinkError::Other("not part of this test".to_owned())) })
}

impl Accountd for Daemon {
    fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
        self.asked.lock().unwrap().push("candidates".to_owned());
        let now = self.offered.lock().unwrap().clone();
        Box::pin(async move { Ok(now) })
    }

    fn token<'a>(&'a self, _: &'a GrantId, _: &'a Audience) -> Answer<'a, IssuedToken> {
        unsaid()
    }

    fn open<'a>(&'a self, _: &'a GrantId, _: &'a ServiceEndpoint) -> Answer<'a, Transport> {
        unsaid()
    }

    fn add_account(&self) -> Answer<'_, AccountId> {
        self.asked.lock().unwrap().push("add_account".to_owned());
        let next = self.adds.lock().unwrap().pop_front();
        // The account the sheet made is offered from now on, if it came with a grant.
        if let Some(Ok((_, Some(offer)))) = &next {
            self.offered.lock().unwrap().push(offer.clone());
        }
        Box::pin(async move {
            match next {
                Some(Ok((id, _))) => Ok(id),
                Some(Err(e)) => Err(e),
                None => Err(LinkError::Refused(Refusal::Dismissed)),
            }
        })
    }

    fn reauthenticate<'a>(&'a self, account: &'a AccountId) -> Answer<'a, ()> {
        self.asked
            .lock()
            .unwrap()
            .push(format!("reauthenticate {account}"));
        Box::pin(async { Ok(()) })
    }

    fn request_grant(&self) -> Answer<'_, Candidate> {
        self.asked.lock().unwrap().push("request_grant".to_owned());
        let next = self.consents.lock().unwrap().pop_front();
        if let Some(Ok(offer)) = &next {
            self.offered.lock().unwrap().push(offer.clone());
        }
        Box::pin(async move { next.unwrap_or(Err(LinkError::Refused(Refusal::Dismissed))) })
    }

    fn revoke<'a>(&'a self, _: &'a GrantId) -> Answer<'a, ()> {
        unsaid()
    }

    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
        Box::pin(async { Ok(None) })
    }
}

/// A change as the follower reacts to it, on a runtime of the test's own.
fn settle(store: &SqliteStore, daemon: &Arc<dyn Accountd>, change: Change) -> Option<Heard> {
    with_runtime(react(store, daemon, change))
}

fn store() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Arc::new(SqliteStore::in_memory(dir.path()).unwrap()), dir)
}

fn addresses(store: &SqliteStore) -> Vec<String> {
    let mut found: Vec<String> = account::linked_accounts(store)
        .into_iter()
        .map(|a| a.address)
        .collect();
    found.sort();
    found
}

#[test]
fn a_change_that_adds_an_account_reads_it_in_and_says_so() {
    let (store, _dir) = store();
    let daemon: Arc<dyn Accountd> =
        Daemon::offering(vec![candidate("fastmail-ada", "ada@example.test", "g-1")]);
    let told = settle(&store, &daemon, Change::Added).expect("something to tell");
    let Heard::Read(read) = &told else {
        panic!("{told:?}")
    };
    assert_eq!(read.added, vec!["ada@example.test".to_owned()]);
    assert!(told.changed());
    assert_eq!(addresses(&store), vec!["ada@example.test".to_owned()]);
    // The same change again finds nothing new, and says so.
    let again = settle(&store, &daemon, Change::Granted).unwrap();
    assert!(!again.changed(), "{again:?}");
}

#[test]
fn a_removal_forgets_that_account_and_no_other() {
    let (store, _dir) = store();
    let daemon: Arc<dyn Accountd> = Daemon::offering(vec![
        candidate("fastmail-ada", "ada@example.test", "g-1"),
        candidate("fastmail-bob", "bob@example.test", "g-2"),
    ]);
    settle(&store, &daemon, Change::Added).unwrap();
    assert_eq!(addresses(&store).len(), 2);

    let told = settle(&store, &daemon, Change::Removed("fastmail_ada".to_owned())).unwrap();
    assert_eq!(told, Heard::Forgotten(vec!["ada@example.test".to_owned()]));
    assert_eq!(addresses(&store), vec!["bob@example.test".to_owned()]);

    // A path that names nobody mailo has is nothing to tell anyone.
    assert_eq!(
        settle(&store, &daemon, Change::Removed("nobody".to_owned())),
        None
    );
    assert_eq!(addresses(&store).len(), 1);
}

#[test]
fn the_follower_reads_and_forgets_as_the_feed_says_and_tells_each_time() {
    struct Feed(Mutex<Option<Listed>>);
    impl std::fmt::Debug for Feed {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Feed")
        }
    }
    #[derive(Debug)]
    struct Following {
        daemon: Arc<Daemon>,
        feed: Feed,
    }
    impl Accountd for Following {
        fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
            self.daemon.candidates()
        }
        fn token<'a>(&'a self, g: &'a GrantId, a: &'a Audience) -> Answer<'a, IssuedToken> {
            self.daemon.token(g, a)
        }
        fn open<'a>(&'a self, g: &'a GrantId, e: &'a ServiceEndpoint) -> Answer<'a, Transport> {
            self.daemon.open(g, e)
        }
        fn add_account(&self) -> Answer<'_, AccountId> {
            self.daemon.add_account()
        }
        fn reauthenticate<'a>(&'a self, a: &'a AccountId) -> Answer<'a, ()> {
            self.daemon.reauthenticate(a)
        }
        fn request_grant(&self) -> Answer<'_, Candidate> {
            self.daemon.request_grant()
        }
        fn revoke<'a>(&'a self, g: &'a GrantId) -> Answer<'a, ()> {
            self.daemon.revoke(g)
        }
        fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
            let feed = self.feed.0.lock().unwrap().take();
            Box::pin(async move { Ok(feed.map(|f| Box::new(f) as Box<dyn Changes>)) })
        }
    }

    let (store, _dir) = store();
    let daemon = Daemon::offering(vec![candidate("fastmail-ada", "ada@example.test", "g-1")]);
    let link = Link::Accountd(Arc::new(Following {
        daemon,
        feed: Feed(Mutex::new(Some(Listed(VecDeque::from([
            Change::Added,
            Change::Removed("fastmail_ada".to_owned()),
        ]))))),
    }));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let told = heard.clone();
    let thread = follow(&store, &link, move |h| told.lock().unwrap().push(h)).expect("a follower");
    thread.join().expect("the follower ends with its feed");
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 2, "{heard:?}");
    assert!(matches!(&heard[0], Heard::Read(read) if read.added == ["ada@example.test"]));
    assert_eq!(
        heard[1],
        Heard::Forgotten(vec!["ada@example.test".to_owned()])
    );
    assert!(addresses(&store).is_empty());
}

#[test]
fn there_is_no_follower_for_an_in_process_link() {
    let (store, _dir) = store();
    assert!(follow(&store, &Link::Local, |_| panic!("nobody to tell")).is_none());
    // And nothing to read: the accounts are mailo's own.
    assert_eq!(read(&store, &Link::Local), Ok(None));
}

#[test]
fn adding_through_the_sheet_reads_the_new_account_in() {
    let (store, _dir) = store();
    let ada = candidate("fastmail-ada", "ada@example.test", "g-1");
    let daemon = Daemon::offering(vec![]);
    // The sheet's "Add, and allow Mail" is one grant: the account is offered at once.
    daemon
        .adds
        .lock()
        .unwrap()
        .push_back(Ok((ada.account.clone(), Some(ada))));
    let accountd: Arc<dyn Accountd> = daemon.clone();
    let added = add(&store, &accountd, None).unwrap();
    let Added::Account(read) = added else {
        panic!("{added:?}")
    };
    assert_eq!(read.added, vec!["ada@example.test".to_owned()]);
    assert!(
        !daemon.asked().contains(&"request_grant".to_owned()),
        "{:?}",
        daemon.asked()
    );
}

#[test]
fn an_account_added_without_mail_allowed_asks_for_the_grant_once() {
    let (store, _dir) = store();
    let ada = candidate("fastmail-ada", "ada@example.test", "g-1");
    let daemon = Daemon::offering(vec![]);
    // Added with no grant for Mail: not offered until the consent sheet says yes.
    daemon
        .adds
        .lock()
        .unwrap()
        .push_back(Ok((ada.account.clone(), None)));
    daemon.consents.lock().unwrap().push_back(Ok(ada));
    let accountd: Arc<dyn Accountd> = daemon.clone();
    let added = add(&store, &accountd, None).unwrap();
    assert!(matches!(added, Added::Account(read) if read.added == ["ada@example.test"]));
    let asked = daemon.asked();
    assert_eq!(
        asked.iter().filter(|a| *a == "request_grant").count(),
        1,
        "{asked:?}"
    );
}

#[test]
fn a_sheet_the_person_closes_is_nothing_to_report() {
    let (store, _dir) = store();
    let daemon = Daemon::offering(vec![]);
    let accountd: Arc<dyn Accountd> = daemon.clone();
    // No answer queued: the stand-in says the sheet was dismissed.
    assert_eq!(add(&store, &accountd, None), Ok(Added::Nothing));
    // And a refusal that is not a closed sheet is an error to say.
    daemon
        .adds
        .lock()
        .unwrap()
        .push_back(Err(LinkError::Unreachable));
    assert!(add(&store, &accountd, None).is_err());
}

#[test]
fn signing_in_again_goes_to_accountd_for_its_accounts_and_for_no_other() {
    let (store, _dir) = store();
    let daemon = Daemon::offering(vec![candidate("fastmail-ada", "ada@example.test", "g-1")]);
    let accountd: Arc<dyn Accountd> = daemon.clone();
    read(&store, &Link::Accountd(accountd.clone())).unwrap();
    let again = add(&store, &accountd, Some("Ada@Example.test")).unwrap();
    assert!(matches!(again, Added::Account(_)), "{again:?}");
    assert!(
        daemon
            .asked()
            .contains(&"reauthenticate fastmail-ada".to_owned()),
        "{:?}",
        daemon.asked()
    );
    assert!(!daemon.asked().contains(&"add_account".to_owned()));
}

#[test]
fn what_a_read_did_is_said_in_one_line_or_not_at_all() {
    assert_eq!(said(&Reconciled::default()), None);
    let line = said(&Reconciled {
        added: vec!["ada@example.test".to_owned()],
        held: vec!["bob@example.test".to_owned()],
        ..Reconciled::default()
    })
    .unwrap();
    assert!(line.contains("added ada@example.test"), "{line}");
    assert!(
        line.contains("bob@example.test is still signed in by Mail itself"),
        "{line}"
    );
    assert!(
        line.contains("Remove it first: mailo account remove bob@example.test"),
        "{line}"
    );
}

/// A row as an unlinked start wrote it: Mail signed this account in itself.
fn held_row(store: &SqliteStore, address: &str) {
    let manual = mail_domain::presets::Manual {
        imap_host: "imap.example.test".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.example.test".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let now = chrono::Utc::now();
    let preset = mail_domain::presets::manual(address, &manual, now);
    let id = mail_domain::id::new_account_id();
    mail_store::testing::seed_account_plan(&store, id.clone(), address, &preset.plan, Some(now));
    mail_store::testing::seed_caps(&store, id.clone(), &preset.expected_caps, now).unwrap();
}

#[test]
fn the_line_is_there_only_when_linked_and_only_when_mail_signed_some_account_in_itself() {
    let accountd: Arc<dyn Accountd> = Daemon::offering(vec![]);
    let linked = Link::Accountd(accountd);

    // Linked, with only accountd's accounts: no line.
    let (store, _dir) = store();
    read(
        &store,
        &Link::Accountd(Daemon::offering(vec![candidate(
            "fastmail-ada",
            "ada@example.test",
            "g-1",
        )])),
    )
    .unwrap();
    hold_back(&store, &linked);
    assert_eq!(held_line(&store), None);

    // Linked, with one Mail signed in itself: the line, and that account is not among the
    // accounts the window draws or the sync takes.
    held_row(&store, "bob@example.test");
    assert_eq!(held_line(&store), Some(HELD_LINE));
    assert_eq!(
        mail_core::sync::addresses(&store)
            .into_iter()
            .map(|(_, address)| address)
            .collect::<Vec<_>>(),
        ["ada@example.test"]
    );

    // Not linked: no line, and the account is Mail's again, untouched.
    hold_back(&store, &Link::Local);
    assert_eq!(held_line(&store), None);
    assert_eq!(mail_core::sync::addresses(&store).len(), 2);
}

#[cfg(all(feature = "quire-desktop", target_os = "linux"))]
mod probe {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    /// A `dbus-daemon` of our own with its own (empty or one-name) service directory, killed by
    /// its pid when this drops. The probe asks this bus and never the person's.
    struct Bus {
        child: Child,
        address: String,
        _dir: tempfile::TempDir,
    }

    impl Bus {
        fn start(activatable: &[&str]) -> Bus {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("services")).unwrap();
            for name in activatable {
                std::fs::write(
                    dir.path().join("services").join(format!("{name}.service")),
                    format!("[D-BUS Service]\nName={name}\nExec=/bin/false\n"),
                )
                .unwrap();
            }
            let socket = dir.path().join("bus");
            let config = dir.path().join("bus.conf");
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
            .unwrap();
            let child = Command::new("dbus-daemon")
                .env_clear()
                .env("HOME", dir.path())
                .env("XDG_RUNTIME_DIR", dir.path())
                .arg(format!("--config-file={}", config.display()))
                .arg("--nofork")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("these tests need dbus-daemon on PATH");
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
                .unwrap()
                .build()
                .await
                .unwrap()
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    async fn presence(bus: &Bus) -> Here {
        let connection = bus.connect().await;
        here(&ds_desktop::Desktop::probe_on(Some(&connection), None).await)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accountd_is_here_when_its_name_has_an_owner_or_the_bus_can_start_it() {
        // Nothing owns the name and nothing can start it: not here, so the process stays in process.
        let empty = Bus::start(&[]);
        assert_eq!(presence(&empty).await, Here::No);

        // A fake owns it.
        let owned = Bus::start(&[]);
        let owner = owned.connect().await;
        owner.request_name("org.quire.Accounts1").await.unwrap();
        assert_eq!(presence(&owned).await, Here::Yes);

        // Nobody owns it, but the bus has a service file for it: activatable is here.
        let activatable = Bus::start(&["org.quire.Accounts1"]);
        assert_eq!(presence(&activatable).await, Here::Yes);

        // Another desktop service being here is not accountd being here.
        let other = Bus::start(&[]);
        let intentd = other.connect().await;
        intentd.request_name("org.quire.Intents1").await.unwrap();
        assert_eq!(presence(&other).await, Here::No);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_probe_feeds_the_choice_and_a_missing_accountd_falls_back_to_in_process() {
        // Here, and the connection works: linked. Here, and it does not: in process.
        let owned = Bus::start(&[]);
        let owner = owned.connect().await;
        owner.request_name("org.quire.Accounts1").await.unwrap();
        let here = presence(&owned).await;
        let linked = link::start_with(here, async {
            Some(link::dbus::over(owned.connect().await, Usage::Interactive))
        })
        .await;
        assert!(linked.is_linked());
        let down = link::start_with(here, async { None }).await;
        assert!(!down.is_linked());
        let absent = Bus::start(&[]);
        let nothing = link::start_with(presence(&absent).await, async {
            panic!("connected though the probe found no accountd")
        })
        .await;
        assert!(!nothing.is_linked());
    }
}
