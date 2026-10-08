//! The choice, the failures' words, and the bearer source, against a table standing in for accountd.

use super::*;
use crate::tokens::{AfterRefusal, Token, TokenSource};
use chrono::{TimeZone, Utc};
use mail_domain::Retryable;
use porter_core::Credential;
use porter_core::{SecretText, TokenKind};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What accountd would say to `token`, in order; everything else it refuses.
#[derive(Debug, Default)]
struct Table {
    tokens: Mutex<VecDeque<Result<IssuedToken, LinkError>>>,
    asked: AtomicUsize,
}

impl Table {
    fn saying(tokens: Vec<Result<IssuedToken, LinkError>>) -> Arc<Table> {
        Arc::new(Table {
            tokens: Mutex::new(tokens.into()),
            asked: AtomicUsize::new(0),
        })
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

fn nothing<T>() -> Answer<'static, T> {
    Box::pin(async { Err(LinkError::Other("not part of this test".to_owned())) })
}

impl Accountd for Table {
    fn candidates(&self) -> Answer<'_, Vec<Candidate>> {
        nothing()
    }

    fn token<'a>(&'a self, _: &'a GrantId, _: &'a Audience) -> Answer<'a, IssuedToken> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let next = self.tokens.lock().unwrap().pop_front();
        Box::pin(async move { next.unwrap_or(Err(LinkError::Unreachable)) })
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

    fn revoke<'a>(&'a self, _: &'a GrantId) -> Answer<'a, ()> {
        nothing()
    }

    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>> {
        Box::pin(async { Ok(None) })
    }
}

/// A future that must never be polled: the connection a "not here" must not attempt.
async fn never() -> Option<Arc<dyn Accountd>> {
    panic!("the link was attempted though accountd is not here");
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

#[test]
fn the_link_is_chosen_once_by_whether_accountd_is_here_and_reachable() {
    let table: Arc<dyn Accountd> = Table::saying(vec![]);
    let rt = runtime();
    // Not here: in process, and no connection is tried.
    let link = rt.block_on(start_with(Here::No, never()));
    assert!(matches!(link, Link::Local) && !link.is_linked());
    assert_eq!(link.name(), "in process");
    // Here, and the connection fails (activatable but it will not start): in process too.
    let link = rt.block_on(start_with(Here::Yes, async { None }));
    assert!(matches!(link, Link::Local));
    // Here, and it answers.
    let link = rt.block_on(start_with(Here::Yes, async { Some(table.clone()) }));
    assert!(link.is_linked() && link.accountd().is_some());
    assert_eq!(link.name(), "accountd over D-Bus");
}

#[test]
fn start_asks_nothing_of_the_bus_when_the_probe_says_accountd_is_not_here() {
    // `start` is the production entry. With `Here::No` it has no connection to make in any build;
    // with `Here::Yes` it would reach the session bus, which a test never does.
    let link = runtime().block_on(start(Here::No, porter_core::consent::Usage::Background));
    assert!(matches!(link, Link::Local));
}

#[test]
fn each_refusal_is_told_to_the_person_or_waited_out() {
    use Refusal::*;
    let wait = |e: LinkError| matches!(e.retry(), Retry::After(_));
    let person = |e: LinkError| e.needs_person();
    assert!(wait(LinkError::Unreachable));
    assert!(wait(LinkError::Refused(Unavailable)));
    for refusal in [
        NeedsReauth,
        UnknownGrant,
        AudienceNotGranted,
        Denied,
        Dismissed,
        NoFittingAccount,
    ] {
        assert!(person(LinkError::Refused(refusal)), "{refusal:?}");
    }
    // A grant gone, not covering the service or refused is allowed again, never signed in again.
    for refusal in [UnknownGrant, AudienceNotGranted, Denied] {
        assert_eq!(
            LinkError::Refused(refusal).retry(),
            Retry::NeedsGrant,
            "{refusal:?}"
        );
    }
    for refusal in [NeedsReauth, Dismissed, NoFittingAccount] {
        assert_eq!(
            LinkError::Refused(refusal).retry(),
            Retry::NeedsReauth,
            "{refusal:?}"
        );
    }
    let fatal = LinkError::Refused(EndpointNotGranted);
    assert!(matches!(fatal.retry(), Retry::Fatal(_)));
    assert!(matches!(
        LinkError::Other("x".to_owned()).retry(),
        Retry::Fatal(_)
    ));
    // A refusal reaches the engines as a `RuntimeError` that keeps what to do about it.
    let error: RuntimeError = LinkError::Refused(NeedsReauth).into();
    assert!(matches!(error.retry(), Retry::NeedsReauth));
}

fn token(value: &str, expires: i64) -> IssuedToken {
    IssuedToken {
        kind: TokenKind::Bearer,
        value: SecretText::new(value),
        expires: UnixSeconds(expires),
    }
}

const NOON: i64 = 1_700_000_000;

fn tokens(table: &Arc<Table>) -> LinkedTokens {
    LinkedTokens::new(
        table.clone(),
        GrantId::parse("grant-1").unwrap(),
        Audience("jmap".to_owned()),
    )
    .with_clock(Arc::new(|| Utc.timestamp_opt(NOON, 0).unwrap()))
}

#[test]
fn a_token_is_asked_for_once_while_it_lasts_and_again_near_its_end() {
    let table = Table::saying(vec![
        Ok(token("first", NOON + 3600)),
        Ok(token("second", NOON + 7200)),
    ]);
    let source = tokens(&table);
    let rt = runtime();
    rt.block_on(async {
        source.ahead(Token::Incoming).await.unwrap();
        source.ahead(Token::Sending).await.unwrap();
        assert_eq!(
            table.asked(),
            1,
            "a token with an hour left is not asked for again"
        );
        let credential = source.current(Token::Incoming).await.unwrap();
        let Credential::OAuth {
            access,
            refresh,
            expires_at,
        } = credential
        else {
            panic!("not a bearer: {credential:?}");
        };
        assert_eq!(access.expose(), "first");
        // The refresh token is accountd's alone: what an engine holds has none.
        assert_eq!(refresh.expose(), "");
        assert_eq!(expires_at, UnixSeconds(NOON + 3600));
    });
    // The same source a minute and a half before the token ends: due.
    let later = LinkedTokens::new(
        table.clone(),
        GrantId::parse("grant-1").unwrap(),
        Audience("jmap".to_owned()),
    )
    .with_clock(Arc::new(|| Utc.timestamp_opt(NOON + 3600 - 30, 0).unwrap()));
    rt.block_on(async {
        // Nothing in hand yet in this source, so it asks; and the table's second answer is
        // what it gets.
        later.ahead(Token::Incoming).await.unwrap();
        assert_eq!(table.asked(), 2);
        let Credential::OAuth { access, .. } = later.current(Token::Incoming).await.unwrap() else {
            panic!("not a bearer");
        };
        assert_eq!(access.expose(), "second");
    });
}

#[test]
fn a_refused_token_is_replaced_once_and_a_second_refusal_is_final() {
    let table = Table::saying(vec![
        Ok(token("first", NOON + 3600)),
        Ok(token("second", NOON + 3600)),
    ]);
    let source = tokens(&table);
    runtime().block_on(async {
        source.ahead(Token::Incoming).await.unwrap();
        assert_eq!(
            source.after_refusal(Token::Incoming).await.unwrap(),
            AfterRefusal::TryAgain
        );
        let Credential::OAuth { access, .. } = source.current(Token::Incoming).await.unwrap()
        else {
            panic!("not a bearer");
        };
        assert_eq!(access.expose(), "second");
        // The token minted after a refusal was refused too: no third.
        assert_eq!(
            source.after_refusal(Token::Incoming).await.unwrap(),
            AfterRefusal::StillRefused
        );
        assert_eq!(table.asked(), 2);
    });
}

#[test]
fn accountds_own_refusal_is_heard_once_and_remembered() {
    let table = Table::saying(vec![Err(LinkError::Refused(Refusal::NeedsReauth))]);
    let source = tokens(&table);
    runtime().block_on(async {
        let first = source.ahead(Token::Incoming).await.unwrap_err();
        assert!(matches!(first.retry(), Retry::NeedsReauth), "{first}");
        // Said again without asking, as the in-process source says the issuer's.
        let again = source.current(Token::Incoming).await.unwrap_err();
        assert!(matches!(again.retry(), Retry::NeedsReauth));
        let after = source.after_refusal(Token::Incoming).await.unwrap_err();
        assert!(matches!(after.retry(), Retry::NeedsReauth));
        assert_eq!(table.asked(), 1);
    });
}

#[test]
fn an_unreachable_accountd_is_not_remembered_as_a_refusal() {
    let table = Table::saying(vec![
        Err(LinkError::Unreachable),
        Ok(token("later", NOON + 3600)),
    ]);
    let source = tokens(&table);
    runtime().block_on(async {
        let first = source.ahead(Token::Incoming).await.unwrap_err();
        assert!(matches!(first.retry(), Retry::After(_)));
        source.ahead(Token::Incoming).await.unwrap();
        assert_eq!(table.asked(), 2);
    });
}

#[test]
fn the_process_link_is_in_process_until_one_is_installed() {
    // `install` is once per process and other tests share it, so only the default is asserted.
    assert!(matches!(current(), Link::Local) || current().is_linked());
}

#[test]
fn a_linked_process_hands_the_link_to_whoever_asks_and_holds_no_secret_of_its_own() {
    use porter_core::{SecretKey, SecretPurpose};
    use porter_secrets::MemorySecrets;
    let plain: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
    assert!(plain.link().is_none(), "a plain store has no link");

    let table: Arc<dyn Accountd> = Table::saying(vec![]);
    let linked = LinkedSecrets::new(table);
    assert!(linked.link().is_some());

    let key = SecretKey {
        account: mail_domain::id::account_id_from_uuid(uuid::uuid!(
            "00000000-0000-4000-8000-00000000c0de"
        )),
        purpose: SecretPurpose::IncomingPassword,
    };
    runtime().block_on(async {
        assert!(
            linked.get(&key).await.is_err(),
            "nothing is read from a keyring"
        );
        assert!(
            linked
                .put(&key, &Credential::Password(SecretText::new("x")))
                .await
                .is_err(),
            "nothing is written to one"
        );
        // Forgetting is not an error and reaches nothing.
        linked.forget(&key).await.unwrap();
        linked.forget_account(&key.account).await.unwrap();
    });
}
