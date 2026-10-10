//! An account of accountd's becomes a row, is read again without losing what the person set, is
//! never put over one mailo holds itself, and is forgotten when accountd removes it.

use crate::account::{
    RemoveError, find_linked, forget, linked_accounts, named_by_segment, preset_of, reconcile,
    remove,
};
use mail_domain::{AccountPlan, AuthPlan, HttpAuth, Incoming, Outgoing, Tls};
use mail_runtime::link::{Accountd, Answer, Changes, LinkError, LinkedSecrets};
use mail_runtime::{AccountSecrets, Transport};
use mail_store::SqliteStore;
use porter_core::capability::{
    Access, Capability, Delta, LabelModel, MailCap, MailTransport, Offered,
};
use porter_core::{
    AccountId, AccountLabel, Audience, Candidate, EndpointUrl, Family, GrantId, IssuedToken,
    LoginName, ProviderId, Restriction, ServiceEndpoint, Subject,
};
use porter_secrets::MemorySecrets;
use std::sync::{Arc, Mutex};

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn endpoint(family: Family, url: &str, tls: porter_core::Tls) -> ServiceEndpoint {
    ServiceEndpoint {
        family,
        url: EndpointUrl::parse(url).unwrap(),
        tls,
        login: LoginName("me@example.test".to_owned()),
    }
}

fn candidate(account: &str, grant: &str, endpoints: Vec<ServiceEndpoint>) -> Candidate {
    Candidate {
        account: AccountId::parse(account).unwrap(),
        label: AccountLabel("me@example.test".to_owned()),
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
        endpoints,
    }
}

fn fastmail(account: &str, grant: &str) -> Candidate {
    candidate(
        account,
        grant,
        vec![
            endpoint(
                Family::Imap,
                "imaps://imap.fastmail.test:993",
                porter_core::Tls::Implicit,
            ),
            endpoint(
                Family::Smtp,
                "smtp://smtp.fastmail.test:587",
                porter_core::Tls::StartTls,
            ),
        ],
    )
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (SqliteStore::in_memory(dir.path()).unwrap(), dir)
}

fn plan_of(store: &SqliteStore, address: &str) -> AccountPlan {
    store
        .account_by_address(address)
        .unwrap()
        .expect("the account")
        .plan
        .unwrap()
}

fn rows(store: &SqliteStore) -> i64 {
    mail_store::testing::count(store, "accounts")
}

#[test]
fn an_imap_candidate_is_a_plan_with_its_servers_and_no_secret() {
    let preset = preset_of(&fastmail("fastmail-me", "grant-1"), now()).unwrap();
    let plan = preset.plan;
    assert_eq!(plan.address, "me@example.test");
    assert_eq!(
        plan.incoming,
        Incoming::Imap {
            host: "imap.fastmail.test".to_owned(),
            port: 993,
            tls: Tls::Implicit
        }
    );
    // Submission as the candidate lists it: STARTTLS on 587 is the relay's, shown as it is.
    assert_eq!(
        plan.outgoing,
        Outgoing::Smtp {
            host: "smtp.fastmail.test".to_owned(),
            port: 587,
            tls: Tls::StartTlsRequired
        }
    );
    let AuthPlan::Granted {
        account,
        grant,
        endpoints,
    } = &plan.auth
    else {
        panic!("not granted: {:?}", plan.auth);
    };
    assert_eq!(account.as_str(), "fastmail-me");
    assert_eq!(grant.as_str(), "grant-1");
    assert_eq!(endpoints.len(), 2);
    assert_eq!(plan.grant(), Some(grant));
    assert_eq!(plan.endpoint(Family::Smtp).unwrap().url.origin().port, 587);
    assert!(plan.endpoint(Family::Sieve).is_none());
}

#[test]
fn jmap_graph_and_pop3_candidates_read_as_what_they_are() {
    let jmap = candidate(
        "fm-jmap",
        "g-1",
        vec![endpoint(
            Family::Jmap,
            "https://api.fastmail.test/jmap/session",
            porter_core::Tls::Implicit,
        )],
    );
    let plan = preset_of(&jmap, now()).unwrap().plan;
    assert_eq!(
        plan.incoming,
        Incoming::Jmap {
            session: "https://api.fastmail.test/jmap/session".to_owned(),
            auth: HttpAuth::Bearer
        }
    );
    let graph = candidate(
        "ms-graph",
        "g-2",
        vec![endpoint(
            Family::Graph,
            "https://graph.microsoft.com",
            porter_core::Tls::Implicit,
        )],
    );
    let plan = preset_of(&graph, now()).unwrap().plan;
    assert_eq!(
        (plan.incoming, plan.outgoing),
        (Incoming::Graph, Outgoing::Graph)
    );
    let pop = candidate(
        "pop-me",
        "g-3",
        vec![endpoint(
            Family::Pop3,
            "pop3s://pop.example.test:995",
            porter_core::Tls::Implicit,
        )],
    );
    let plan = preset_of(&pop, now()).unwrap().plan;
    assert!(matches!(plan.incoming, Incoming::Pop3 { port: 995, .. }));
    // The grant lists no SMTP server: it receives only, and does not pretend to send.
    assert_eq!(plan.outgoing, Outgoing::Nowhere);
}

#[test]
fn a_candidate_with_no_mail_server_or_no_address_is_said_not_to_be_mail() {
    let none = candidate("calendar-only", "g-4", vec![]);
    let why = preset_of(&none, now()).unwrap_err().to_string();
    assert!(why.contains("lists no IMAP"), "{why}");
    let mut nameless = fastmail("fastmail-x", "g-5");
    nameless.label = AccountLabel("Work".to_owned());
    for e in &mut nameless.endpoints {
        e.login = LoginName("workuser".to_owned());
    }
    let why = preset_of(&nameless, now()).unwrap_err().to_string();
    assert!(why.contains("names no address"), "{why}");
}

#[test]
fn reading_the_accounts_adds_each_once_and_keeps_what_the_person_set() {
    let (store, _dir) = store();
    let candidates = vec![fastmail("fastmail-me", "grant-1")];
    let first = reconcile(&store, &candidates, now()).unwrap();
    assert_eq!(first.added, vec!["me@example.test".to_owned()]);
    assert_eq!(rows(&store), 1);
    // The row's id is mailo's own (a UUID: every table of the store reads its account column as
    // one), and accountd's name is in the plan.
    let here = &linked_accounts(&store)[0];
    assert!(uuid::Uuid::parse_str(here.id.as_str()).is_ok(), "{here:?}");
    assert_eq!(here.account.as_str(), "fastmail-me");
    // Expected capabilities were written for the first sync to start from.
    let caps: i64 = mail_store::testing::count(&store, "account_caps");
    assert_eq!(caps, 1);

    // Read again unchanged: nothing is added or updated.
    let again = reconcile(&store, &candidates, now()).unwrap();
    assert!(
        again.added.is_empty() && again.updated.is_empty(),
        "{again:?}"
    );

    // The person sets a signature; accountd then gives Mail a new grant (the account was allowed
    // again). The grant is brought up to date and the signature is still there.
    let account = store.list_accounts().unwrap().remove(0).id;
    let mine = store.identities(account.clone()).unwrap().remove(0);
    store
        .set_signature(mine.id, Some("Sent from here"))
        .unwrap();
    let again = reconcile(&store, &[fastmail("fastmail-me", "grant-2")], now()).unwrap();
    assert_eq!(again.updated, vec!["me@example.test".to_owned()]);
    assert_eq!(rows(&store), 1);
    let plan = plan_of(&store, "me@example.test");
    assert_eq!(plan.grant().unwrap().as_str(), "grant-2");
    assert_eq!(plan.identities.len(), 1);
    let kept = store.identities(account).unwrap().remove(0).signature;
    assert_eq!(kept.as_deref(), Some("Sent from here"));
}

#[test]
fn an_address_mailo_holds_itself_is_left_as_it_is() {
    let (store, _dir) = store();
    mail_store::testing::seed_account(
        &store,
        porter_core::AccountId::parse("00000000-0000-4000-8000-0000000000a1").unwrap(),
        "me@example.test",
    );
    let said = reconcile(&store, &[fastmail("fastmail-me", "grant-1")], now()).unwrap();
    assert!(said.added.is_empty());
    assert_eq!(said.held, vec!["me@example.test".to_owned()]);
    assert_eq!(rows(&store), 1);
    let stored = store
        .account_by_address("me@example.test")
        .unwrap()
        .expect("the account");
    assert!(
        stored.plan.is_err(),
        "the account held with its own sign-in was touched"
    );
}

#[test]
fn an_account_that_cannot_be_mail_is_reported_and_the_rest_are_read() {
    let (store, _dir) = store();
    let said = reconcile(
        &store,
        &[
            candidate("calendar-only", "g-4", vec![]),
            fastmail("fastmail-me", "g-1"),
        ],
        now(),
    )
    .unwrap();
    assert_eq!(said.added.len(), 1);
    assert_eq!(said.unusable.len(), 1);
    assert_eq!(said.unusable[0].0, "me@example.test");
}

#[test]
fn removing_at_accountd_forgets_the_rows_by_the_signals_path_segment() {
    let (store, _dir) = store();
    reconcile(
        &store,
        &[fastmail("fastmail-me", "g-1"), {
            let mut other = fastmail("fastmail-other", "g-2");
            other.label = AccountLabel("other@example.test".to_owned());
            for e in &mut other.endpoints {
                e.login = LoginName("other@example.test".to_owned());
            }
            other
        }],
        now(),
    )
    .unwrap();
    assert_eq!(rows(&store), 2);
    // `AccountRemoved` carries only the object path's last segment: `_` for `-`.
    let gone = named_by_segment(&store, "fastmail_me");
    assert_eq!(gone.len(), 1);
    assert_eq!(gone[0].address, "me@example.test");
    assert_eq!(
        find_linked(&store, &AccountId::parse("fastmail-other").unwrap())
            .unwrap()
            .address,
        "other@example.test"
    );
    let done = forget(&store, &gone);
    assert!(done[0].1.is_ok());
    assert_eq!(rows(&store), 1);
    assert!(named_by_segment(&store, "fastmail_me").is_empty());
}

/// What accountd would be told by a removal.
#[derive(Debug, Default)]
struct Revoking {
    revoked: Mutex<Vec<String>>,
    refuse: bool,
}

fn nothing<T>() -> Answer<'static, T> {
    Box::pin(async { Err(LinkError::Other("not part of this test".to_owned())) })
}

impl Accountd for Revoking {
    fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
        nothing()
    }
    fn token<'a>(&'a self, _: &'a GrantId, _: &'a Audience) -> Answer<'a, IssuedToken> {
        nothing()
    }
    fn open<'a>(&'a self, _: &'a GrantId, _: &'a ServiceEndpoint) -> Answer<'a, Transport> {
        nothing()
    }
    fn add_account(&self) -> Answer<'_, AccountId> {
        nothing()
    }
    fn reauthenticate<'a>(&'a self, _: &'a AccountId) -> Answer<'a, ()> {
        nothing()
    }
    fn request_grant(&self) -> Answer<'_, Candidate> {
        nothing()
    }
    fn revoke<'a>(&'a self, grant: &'a GrantId) -> Answer<'a, ()> {
        let refuse = self.refuse;
        self.revoked.lock().unwrap().push(grant.to_string());
        Box::pin(async move {
            if refuse {
                Err(LinkError::Unreachable)
            } else {
                Ok(())
            }
        })
    }
    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
        Box::pin(async { Ok(None) })
    }
}

#[test]
fn removing_a_linked_account_withdraws_the_grant_first_and_touches_no_keyring() {
    let (store, _dir) = store();
    reconcile(&store, &[fastmail("fastmail-me", "grant-1")], now()).unwrap();
    let id = linked_accounts(&store)[0].id.clone();

    // accountd cannot be reached: nothing is removed, the account and its mail are still here.
    let down = Arc::new(Revoking {
        refuse: true,
        ..Revoking::default()
    });
    let secrets = LinkedSecrets::new(down.clone());
    let refused = mail_runtime::block_on(remove(&store, &secrets, id.clone()));
    assert!(
        matches!(refused, Err(RemoveError::Keyring(_))),
        "{refused:?}"
    );
    assert_eq!(rows(&store), 1);

    // Reached: the grant is withdrawn, then the rows go.
    let up = Arc::new(Revoking::default());
    let secrets = LinkedSecrets::new(up.clone());
    let removed = mail_runtime::block_on(remove(&store, &secrets, id)).unwrap();
    assert_eq!(removed.address, "me@example.test");
    assert_eq!(
        up.revoked.lock().unwrap().as_slice(),
        ["grant-1".to_owned()]
    );
    assert_eq!(rows(&store), 0);

    // No link at all (a plain store): the same account is not removed on a guess.
    reconcile(&store, &[fastmail("fastmail-me", "grant-1")], now()).unwrap();
    let id = linked_accounts(&store)[0].id.clone();
    let plain: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
    let refused = mail_runtime::block_on(remove(&store, plain.as_ref(), id));
    assert!(matches!(refused, Err(RemoveError::Keyring(_))));
    assert_eq!(rows(&store), 1);
}
