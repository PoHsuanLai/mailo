//! The mail sign-in as the machine drives it, over fakes that count: look only when asked, add
//! only when told to, a password to the add and nowhere else, a browser sign-in cancelled by
//! dropping it.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use mail_core::account::Setup;
use mail_core::discover::{Failed, Found, Gap, Source};
use mail_domain::presets::{self, Manual};
use mail_domain::{HttpAuth, Incoming, LeaveOnServer, Outgoing, Tls};
use porter_core::sheet::{
    Entry, FieldAnswer, FieldKind, FieldSpec, FieldValue, Presence, Protocol, SignInFault,
    SignInInput, manual_form,
};
use porter_core::{Credential, ProviderId, SecretText};
use porter_provider::{Issuer, Provider, SignIn, SignInMode, SignInStart, SignInStep};
use tokio::sync::Notify;

use super::provider::{Added, MailProvider, Request, Seams, offered};

pub(super) const PASSWORD: &str = "s3cret-pass-4417";
pub(super) const AUTHORIZE: &str =
    "https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz";

/// What the fakes were asked.
#[derive(Default)]
pub(super) struct Log {
    pub lookups: Mutex<Vec<String>>,
    pub jmaps: Mutex<Vec<String>>,
    pub adds: Mutex<Vec<(String, Option<String>, bool)>>,
}

/// How each seam answers.
#[derive(Clone)]
pub(super) struct Script {
    pub lookup: Result<(), Failed>,
    pub jmap: Result<String, String>,
    pub add: Result<String, String>,
    pub client: bool,
    /// A browser sign-in finishes when this is notified; `None` refuses it.
    pub release: Arc<Notify>,
    pub signed_in: Result<(), String>,
    /// A lookup waits while a test holds this, to look at the step that is working.
    pub hold: Arc<Mutex<()>>,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            lookup: Ok(()),
            jmap: Err("no jmap".to_owned()),
            add: Ok("added".to_owned()),
            client: true,
            release: Arc::new(Notify::new()),
            signed_in: Ok(()),
            hold: Arc::default(),
        }
    }
}

pub(super) fn imap_found(address: &str) -> Found {
    let manual = Manual {
        imap_host: "imap.example.test".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.example.test".to_owned(),
        smtp_port: 465,
        login: None,
    };
    Found {
        source: Source::Autoconfig,
        preset: presets::manual(address, &manual, Utc::now()),
    }
}

/// Seams that reach nothing, answering as `script` says and counting in `log`.
pub(super) fn seams(script: &Script, log: &Arc<Log>) -> Seams {
    let (s, l) = (script.clone(), log.clone());
    let lookup = move |address: &str| {
        let _held = s.hold.lock().unwrap();
        l.lookups.lock().unwrap().push(address.to_owned());
        s.lookup.clone().map(|()| imap_found(address))
    };
    let (s, l) = (script.clone(), log.clone());
    let jmap = move |url: &str| {
        l.jmaps.lock().unwrap().push(url.to_owned());
        s.jmap.clone()
    };
    let client = script.client;
    let s = script.clone();
    let l = log.clone();
    let add = move |request: Request| {
        let Request {
            address,
            password,
            signed,
            ..
        } = request;
        l.adds.lock().unwrap().push((
            address,
            password.map(|password| password.expose().to_owned()),
            signed.is_some(),
        ));
        s.add.clone()
    };
    let s = script.clone();
    Seams {
        lookup: Arc::new(lookup),
        jmap: Arc::new(jmap),
        client: Arc::new(move |_| client),
        authorize: Arc::new(move |_, _, urls| {
            let s = s.clone();
            Box::pin(async move {
                urls(AUTHORIZE);
                s.release.notified().await;
                s.signed_in
                    .clone()
                    .map(|()| Credential::Password(SecretText::new("refresh-token")))
            })
        }),
        add: Arc::new(add),
    }
}

pub(super) fn provider(
    id: &str,
    seams: &Seams,
    added: &Added,
    prefill: Option<&str>,
) -> MailProvider {
    let id = ProviderId::parse(id).unwrap();
    offered(seams, added, prefill.map(str::to_owned))
        .into_iter()
        .find(|provider| provider.spec().id == id)
        .unwrap_or_else(|| panic!("{id} is not offered"))
}

fn start(provider: &MailProvider) -> super::provider::MailSignIn {
    provider
        .sign_in(SignInStart {
            mode: SignInMode::Add,
        })
        .unwrap()
}

fn answer(kind: FieldKind, value: FieldValue) -> FieldAnswer {
    FieldAnswer { kind, value }
}

fn address(text: &str) -> FieldAnswer {
    answer(FieldKind::Address, FieldValue::Plain(text.to_owned()))
}

fn password(text: &str) -> FieldAnswer {
    answer(
        FieldKind::Password,
        FieldValue::Secret(SecretText::new(text)),
    )
}

#[test]
fn the_providers_offered_are_mails_own_in_the_order_the_list_shows_them() {
    let log = Arc::new(Log::default());
    let all = offered(&seams(&Script::default(), &log), &Added::default(), None);
    let ids: Vec<&str> = all.iter().map(|p| p.spec().id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "google",
            "microsoft",
            "fastmail",
            "icloud",
            "yahoo",
            "gmx",
            "generic-imap"
        ]
    );
}

#[tokio::test]
async fn a_password_form_asks_for_an_address_and_a_password_and_an_oauth_one_for_the_address() {
    let log = Arc::new(Log::default());
    let fakes = seams(&Script::default(), &log);
    for (id, kinds) in [
        (
            "generic-imap",
            vec![FieldKind::Address, FieldKind::Password],
        ),
        ("fastmail", vec![FieldKind::Address, FieldKind::Password]),
        ("google", vec![FieldKind::Address]),
        ("microsoft", vec![FieldKind::Address]),
    ] {
        let mut sign_in = start(&provider(
            id,
            &fakes,
            &Added::default(),
            Some("ada@example.test"),
        ));
        let SignInStep::AskFields(fields) = sign_in.next(SignInInput::Start).await else {
            panic!("{id} did not ask");
        };
        assert_eq!(
            fields.iter().map(|f| f.kind).collect::<Vec<_>>(),
            kinds,
            "{id}"
        );
        assert!(
            fields.iter().all(|f| f.presence == Presence::Required),
            "{id}"
        );
        assert_eq!(
            fields[0].prefill.as_deref(),
            Some("ada@example.test"),
            "{id}"
        );
        assert_eq!(
            fields.last().map(|f| f.entry),
            Some(if kinds.len() == 2 {
                Entry::Secret
            } else {
                Entry::Plain
            })
        );
    }
    assert!(
        log.lookups.lock().unwrap().is_empty(),
        "nothing is looked up before it is asked"
    );
}

#[tokio::test]
async fn a_password_account_is_looked_up_once_reviewed_and_added_only_on_confirm() {
    let log = Arc::new(Log::default());
    let added = Added::default();
    let fakes = seams(&Script::default(), &log);
    let mut sign_in = start(&provider("generic-imap", &fakes, &added, None));
    sign_in.next(SignInInput::Start).await;
    let review = sign_in
        .next(SignInInput::Fields(vec![
            address("Ada@Example.test"),
            password(PASSWORD),
        ]))
        .await;
    let SignInStep::Review { label, claims, .. } = review else {
        panic!("{review:?}");
    };
    assert_eq!(label.0, "ada@example.test");
    assert_eq!(claims.len(), 1, "only Mail is offered to switch");
    assert_eq!(*log.lookups.lock().unwrap(), ["ada@example.test"]);
    assert!(
        log.adds.lock().unwrap().is_empty(),
        "nothing is added before Confirm"
    );

    let done = sign_in.next(SignInInput::Confirm(vec![])).await;
    assert!(matches!(done, SignInStep::Done(_)), "{done:?}");
    let adds = log.adds.lock().unwrap();
    assert_eq!(
        *adds,
        [(
            "ada@example.test".to_owned(),
            Some(PASSWORD.to_owned()),
            false
        )]
    );
    assert_eq!(added.take().as_deref(), Some("ada@example.test"));
    let SignInStep::Done(signed) = done else {
        unreachable!()
    };
    assert!(
        signed.credentials.is_empty(),
        "porter's store is handed no secret"
    );
    assert!(!format!("{signed:?}{sign_in:?}").contains(PASSWORD));
}

#[tokio::test]
async fn an_oauth_provider_asks_for_a_password_only_when_the_address_turns_out_to_want_one() {
    let log = Arc::new(Log::default());
    let fakes = seams(&Script::default(), &log);
    let mut sign_in = start(&provider("google", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    let again = sign_in
        .next(SignInInput::Fields(vec![address("ada@example.test")]))
        .await;
    let SignInStep::AskFields(fields) = again else {
        panic!("{again:?}");
    };
    assert_eq!(
        fields.iter().map(|f| f.kind).collect::<Vec<_>>(),
        [FieldKind::Address, FieldKind::Password]
    );
    assert_eq!(fields[0].prefill.as_deref(), Some("ada@example.test"));
}

/// No server is found for `ada@example.test`.
fn nothing_found() -> Script {
    Script {
        lookup: Err(Failed::NoServers {
            address: "ada@example.test".to_owned(),
            gap: Gap::Nothing,
            tried: String::new(),
        }),
        ..Script::default()
    }
}

fn plain(kind: FieldKind, text: &str) -> FieldAnswer {
    answer(kind, FieldValue::Plain(text.to_owned()))
}

/// A sign-in that asked for the servers (the lookup found none), over seams whose add records
/// the request it is handed.
async fn asked_servers(
    script: &Script,
) -> (
    super::provider::MailSignIn,
    Vec<FieldSpec>,
    Arc<Mutex<Option<(Setup, Option<String>)>>>,
) {
    let log = Arc::new(Log::default());
    let seen = Arc::new(Mutex::new(None));
    let record = seen.clone();
    let mut fakes = seams(script, &log);
    fakes.add = Arc::new(move |request| {
        *record.lock().unwrap() = Some((
            request.setup,
            request
                .password
                .map(|password| password.expose().to_owned()),
        ));
        Ok(String::new())
    });
    let mut sign_in = start(&provider("generic-imap", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    let ask = sign_in
        .next(SignInInput::Fields(vec![
            address("ada@example.test"),
            password(PASSWORD),
        ]))
        .await;
    let SignInStep::AskFields(fields) = ask else {
        panic!("{ask:?}");
    };
    (sign_in, fields, seen)
}

#[tokio::test]
async fn no_servers_found_asks_for_the_typed_server_form_and_adds_imap_and_smtp_there() {
    let (mut sign_in, fields, seen) = asked_servers(&nothing_found()).await;
    // Porter's form, guessed from the address's domain, with no password asked twice.
    assert_eq!(
        fields,
        manual_form(Protocol::Imap, Some("example.test")),
        "the form is porter's"
    );
    assert!(fields.iter().all(|f| f.kind != FieldKind::Password));
    let review = sign_in
        .next(SignInInput::Fields(vec![
            plain(FieldKind::Protocol, "imap"),
            plain(FieldKind::Server, "mail.example.test"),
            plain(FieldKind::Security, "tls"),
            plain(FieldKind::Port, ""),
            plain(FieldKind::OutgoingServer, "smtp.example.test"),
            plain(FieldKind::OutgoingSecurity, "tls"),
            plain(FieldKind::OutgoingPort, ""),
            plain(FieldKind::Username, ""),
        ]))
        .await;
    assert!(matches!(review, SignInStep::Review { .. }), "{review:?}");
    sign_in.next(SignInInput::Confirm(vec![])).await;
    let (setup, secret) = seen.lock().unwrap().clone().expect("added");
    let manual = Manual {
        imap_host: "mail.example.test".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.example.test".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let want = Setup::Discovered(Box::new(presets::manual(
        "ada@example.test",
        &manual,
        Utc::now(),
    )));
    // `expected_caps.observed_at` is the moment it was made: compare the plan.
    match (&setup, &want) {
        (Setup::Discovered(got), Setup::Discovered(want)) => assert_eq!(got.plan, want.plan),
        _ => panic!("{setup:?}"),
    }
    assert_eq!(secret.as_deref(), Some(PASSWORD));
}

#[tokio::test]
async fn a_typed_pop3_server_with_starttls_and_a_login_is_added_as_pop3_with_each_security() {
    let (mut sign_in, _, seen) = asked_servers(&nothing_found()).await;
    let review = sign_in
        .next(SignInInput::Fields(vec![
            plain(FieldKind::Protocol, "pop3"),
            plain(FieldKind::Server, "pop.example.test"),
            plain(FieldKind::Security, "starttls"),
            plain(FieldKind::Port, "2110"),
            plain(FieldKind::OutgoingServer, "smtp.example.test"),
            plain(FieldKind::OutgoingSecurity, "tls"),
            plain(FieldKind::OutgoingPort, ""),
            plain(FieldKind::Username, "ada"),
        ]))
        .await;
    assert!(matches!(review, SignInStep::Review { .. }), "{review:?}");
    sign_in.next(SignInInput::Confirm(vec![])).await;
    let (setup, secret) = seen.lock().unwrap().clone().expect("added");
    let Setup::Discovered(preset) = setup else {
        panic!("{setup:?}");
    };
    assert_eq!(
        preset.plan.incoming,
        Incoming::Pop3 {
            host: "pop.example.test".to_owned(),
            port: 2110,
            tls: Tls::StartTlsRequired,
            leave: LeaveOnServer::Keep,
        }
    );
    assert_eq!(
        preset.plan.outgoing,
        Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        }
    );
    assert_eq!(preset.plan.username(), "ada");
    assert_eq!(secret.as_deref(), Some(PASSWORD));
}

#[tokio::test]
async fn a_typed_jmap_server_with_a_token_signs_in_with_the_token_as_a_bearer() {
    let (mut sign_in, _, seen) = asked_servers(&nothing_found()).await;
    let review = sign_in
        .next(SignInInput::Fields(vec![
            plain(FieldKind::Protocol, "jmap"),
            plain(FieldKind::SessionUrl, "https://jmap.example.test/session"),
            answer(
                FieldKind::Token,
                FieldValue::Secret(SecretText::new("api-token-77")),
            ),
            plain(FieldKind::Username, ""),
        ]))
        .await;
    assert!(matches!(review, SignInStep::Review { .. }), "{review:?}");
    sign_in.next(SignInInput::Confirm(vec![])).await;
    let (setup, secret) = seen.lock().unwrap().clone().expect("added");
    assert_eq!(
        setup,
        Setup::Jmap {
            session: Some("https://jmap.example.test/session".to_owned()),
            login: None,
            auth: HttpAuth::Bearer,
        }
    );
    assert_eq!(
        secret.as_deref(),
        Some("api-token-77"),
        "the token, not the password"
    );

    // Without a token, the password typed first is the credential, sent as Basic.
    let (mut sign_in, _, seen) = asked_servers(&nothing_found()).await;
    sign_in
        .next(SignInInput::Fields(vec![
            plain(FieldKind::Protocol, "jmap"),
            plain(FieldKind::SessionUrl, "https://jmap.example.test/session"),
            plain(FieldKind::Username, "ada"),
        ]))
        .await;
    sign_in.next(SignInInput::Confirm(vec![])).await;
    let (setup, secret) = seen.lock().unwrap().clone().expect("added");
    assert_eq!(
        setup,
        Setup::Jmap {
            session: Some("https://jmap.example.test/session".to_owned()),
            login: Some("ada".to_owned()),
            auth: HttpAuth::Basic,
        }
    );
    assert_eq!(secret.as_deref(), Some(PASSWORD));
}

#[tokio::test]
async fn each_way_a_lookup_fails_is_its_own_fault() {
    for (failed, fault) in [
        (
            Failed::NoServers {
                address: "a@b.test".to_owned(),
                gap: Gap::PersonalMicrosoft,
                tried: String::new(),
            },
            SignInFault::Forbidden,
        ),
        (
            Failed::Unreachable {
                address: "a@b.test".to_owned(),
                retry: mail_domain::Retry::Now,
                why: "offline".to_owned(),
            },
            SignInFault::Unreachable,
        ),
        (
            Failed::Broken("no runtime".to_owned()),
            SignInFault::Unreachable,
        ),
    ] {
        let log = Arc::new(Log::default());
        let script = Script {
            lookup: Err(failed),
            ..Script::default()
        };
        let fakes = seams(&script, &log);
        let mut sign_in = start(&provider("generic-imap", &fakes, &Added::default(), None));
        sign_in.next(SignInInput::Start).await;
        let step = sign_in
            .next(SignInInput::Fields(vec![
                address("ada@example.test"),
                password("x"),
            ]))
            .await;
        assert_eq!(step, SignInStep::Failed(fault));
    }
}

#[tokio::test]
async fn what_is_not_an_address_is_never_looked_up() {
    let log = Arc::new(Log::default());
    let fakes = seams(&Script::default(), &log);
    let mut sign_in = start(&provider("generic-imap", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    let step = sign_in
        .next(SignInInput::Fields(vec![
            address("not an address"),
            password("x"),
        ]))
        .await;
    assert_eq!(step, SignInStep::Failed(SignInFault::Refused));
    assert!(log.lookups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn jmap_found_alone_is_added_with_its_session_and_the_autoconfig_wins_when_both_answer() {
    let log = Arc::new(Log::default());
    let alone = Script {
        lookup: Err(Failed::Broken("x".to_owned())),
        jmap: Ok("https://jmap.example.test/session".to_owned()),
        ..Script::default()
    };
    let seen = Arc::new(Mutex::new(None));
    let mut fakes = seams(&alone, &log);
    let record = seen.clone();
    fakes.add = Arc::new(move |request| {
        *record.lock().unwrap() = Some(request.setup);
        Ok(String::new())
    });
    let mut sign_in = start(&provider("generic-imap", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    sign_in
        .next(SignInInput::Fields(vec![
            address("ada@example.test"),
            password("pw"),
        ]))
        .await;
    sign_in.next(SignInInput::Confirm(vec![])).await;
    assert!(matches!(
        seen.lock().unwrap().clone(),
        Some(Setup::Jmap { session: Some(session), .. }) if session == "https://jmap.example.test/session"
    ));

    let both = Script {
        jmap: Ok("https://jmap.example.test/session".to_owned()),
        ..Script::default()
    };
    let mut fakes = seams(&both, &log);
    let record = seen.clone();
    fakes.add = Arc::new(move |request| {
        *record.lock().unwrap() = Some(request.setup);
        Ok(String::new())
    });
    let mut sign_in = start(&provider("generic-imap", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    sign_in
        .next(SignInInput::Fields(vec![
            address("ada@example.test"),
            password("pw"),
        ]))
        .await;
    sign_in.next(SignInInput::Confirm(vec![])).await;
    assert!(matches!(
        seen.lock().unwrap().clone(),
        Some(Setup::Discovered(_))
    ));
}

#[tokio::test]
async fn a_refused_add_ends_the_sign_in_as_one_that_could_not_be_stored() {
    let log = Arc::new(Log::default());
    let script = Script {
        add: Err("cannot save the account".to_owned()),
        ..Script::default()
    };
    let added = Added::default();
    let fakes = seams(&script, &log);
    let mut sign_in = start(&provider("generic-imap", &fakes, &added, None));
    sign_in.next(SignInInput::Start).await;
    sign_in
        .next(SignInInput::Fields(vec![
            address("ada@example.test"),
            password("pw"),
        ]))
        .await;
    let step = sign_in.next(SignInInput::Confirm(vec![])).await;
    assert_eq!(step, SignInStep::Failed(SignInFault::StoreFailed));
    assert_eq!(added.take(), None);
}

#[tokio::test]
async fn a_browser_account_opens_the_browser_then_reviews_and_adds_with_the_credential() {
    let log = Arc::new(Log::default());
    let script = Script::default();
    let added = Added::default();
    let fakes = seams(&script, &log);
    let mut sign_in = start(&provider("google", &fakes, &added, None));
    sign_in.next(SignInInput::Start).await;
    let step = sign_in
        .next(SignInInput::Fields(vec![address("ada@gmail.com")]))
        .await;
    let SignInStep::OpenBrowser { url } = step else {
        panic!("{step:?}");
    };
    assert_eq!(url.as_str(), AUTHORIZE);
    assert!(
        log.lookups.lock().unwrap().is_empty(),
        "the built-in table needs no lookup"
    );
    script.release.notify_one();
    let review = sign_in.next(SignInInput::Poll).await;
    assert!(matches!(review, SignInStep::Review { .. }), "{review:?}");
    sign_in.next(SignInInput::Confirm(vec![])).await;
    assert_eq!(
        *log.adds.lock().unwrap(),
        [("ada@gmail.com".to_owned(), None, true)]
    );
}

#[tokio::test]
async fn a_browser_account_without_a_client_id_says_so_and_opens_nothing() {
    let log = Arc::new(Log::default());
    let script = Script {
        client: false,
        ..Script::default()
    };
    let fakes = seams(&script, &log);
    let mut sign_in = start(&provider("google", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    let step = sign_in
        .next(SignInInput::Fields(vec![address("ada@gmail.com")]))
        .await;
    assert_eq!(step, SignInStep::Failed(SignInFault::NeedsClientId));
}

#[tokio::test]
async fn a_refused_browser_sign_in_fails_and_cancel_stops_one_in_flight() {
    let log = Arc::new(Log::default());
    let script = Script {
        signed_in: Err("declined".to_owned()),
        ..Script::default()
    };
    let fakes = seams(&script, &log);
    let mut sign_in = start(&provider("google", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    sign_in
        .next(SignInInput::Fields(vec![address("ada@gmail.com")]))
        .await;
    script.release.notify_one();
    assert_eq!(
        sign_in.next(SignInInput::Poll).await,
        SignInStep::Failed(SignInFault::Refused)
    );

    let script = Script::default();
    let fakes = seams(&script, &log);
    let mut sign_in = start(&provider("google", &fakes, &Added::default(), None));
    sign_in.next(SignInInput::Start).await;
    sign_in
        .next(SignInInput::Fields(vec![address("ada@gmail.com")]))
        .await;
    assert_eq!(
        sign_in.next(SignInInput::Cancel).await,
        SignInStep::Failed(SignInFault::Cancelled)
    );
    // Nothing is waiting any more: a late answer finds no one.
    script.release.notify_one();
    assert_eq!(log.adds.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn the_issuers_the_providers_sign_in_with_are_google_and_microsoft() {
    let log = Arc::new(Log::default());
    let fakes = seams(&Script::default(), &log);
    let google = provider("google", &fakes, &Added::default(), None);
    let microsoft = provider("microsoft", &fakes, &Added::default(), None);
    assert_eq!(google.spec().auth.issuer, Some(Issuer::Google));
    assert_eq!(microsoft.spec().auth.issuer, Some(Issuer::Microsoft));
}
