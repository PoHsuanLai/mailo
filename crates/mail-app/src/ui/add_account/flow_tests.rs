//! The sheet's rules without the sheet: look only when asked, add only when told to, and a
//! password to the add and nowhere else. The network and the keyring are fakes that count.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mail_domain::presets::{Manual, manual};
use mail_domain::{AccountId, OAuthIssuer, SecretKey, SecretPurpose};
use mail_proto::discover::{Found, Source};
use mail_runtime::{MapSecrets, OAuthRegistry, Secrets};
use mail_store::SqliteStore;

use super::flow::{self, Client, Offer, Seams, SignIn, Stage};
use crate::password::Password;
use crate::space::{Scope, Space};

pub(super) fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

/// What an ISPDB answer for `address` would be: IMAP and SMTP with implicit TLS.
pub(super) fn found(address: &str) -> Found {
    Found {
        source: Source::Ispdb,
        preset: manual(
            address,
            &Manual {
                imap_host: "imap.example.test".to_owned(),
                imap_port: 993,
                smtp_host: "smtp.example.test".to_owned(),
                smtp_port: 465,
                login: None,
            },
            now(),
        ),
    }
}

/// Fakes that count: how often each seam was used, and every password the add was handed.
#[derive(Default)]
pub(super) struct Fake {
    pub looked: AtomicUsize,
    pub added: AtomicUsize,
    /// Copied out by the fake add, the one place a test may hold the password.
    pub handed: Mutex<Vec<String>>,
    /// Where the fake add keeps credentials: `add_with_password` into a map, never the keyring.
    pub secrets: MapSecrets,
    /// Every address the browser fake was asked to open. No browser is.
    pub browsed: Mutex<Vec<String>>,
    /// Every URL the JMAP search was handed.
    pub searched: Mutex<Vec<String>>,
    /// Every setup the add was handed.
    pub setups: Mutex<Vec<crate::cli::Setup>>,
}

impl Fake {
    pub(super) fn looked(&self) -> usize {
        self.looked.load(Ordering::SeqCst)
    }

    pub(super) fn added(&self) -> usize {
        self.added.load(Ordering::SeqCst)
    }

    /// The password the keyring fake holds for `account`, if any.
    pub(super) fn kept(&self, account: AccountId) -> Option<String> {
        match self.secrets.get(&SecretKey {
            account,
            purpose: SecretPurpose::IncomingPassword,
        }) {
            Ok(mail_domain::Credential::Password(password)) => Some(password),
            _ => None,
        }
    }
}

/// What a domain with no JMAP server answers at its well-known URL.
pub(super) const NO_JMAP: &str = "https://example.test/.well-known/jmap answered 404 Not Found";

/// Seams over `fake`: lookups answer `answer`, the domain has no JMAP server, adds run the real
/// add into the fake keyring, an OAuth client is at hand when `client` says so, and the browser
/// only writes down its address.
pub(super) fn seams(fake: &Arc<Fake>, answer: Result<Found, String>, client: bool) -> Seams {
    with_jmap(fake, answer, Err(NO_JMAP.to_owned()), client)
}

/// [`seams`], with the JMAP search answering `jmap`.
pub(super) fn with_jmap(
    fake: &Arc<Fake>,
    answer: Result<Found, String>,
    jmap: Result<String, String>,
    client: bool,
) -> Seams {
    let looking = fake.clone();
    let searching = fake.clone();
    let adding = fake.clone();
    let browsing = fake.clone();
    Seams {
        lookup: Arc::new(move |_| {
            looking.looked.fetch_add(1, Ordering::SeqCst);
            answer.clone()
        }),
        jmap: Arc::new(move |url| {
            searching.searched.lock().unwrap().push(url.to_owned());
            jmap.clone()
        }),
        add: Arc::new(move |store, request, on_url| {
            adding.added.fetch_add(1, Ordering::SeqCst);
            if let Some(password) = &request.password {
                adding
                    .handed
                    .lock()
                    .unwrap()
                    .push(password.expose().to_owned());
            }
            adding.setups.lock().unwrap().push(request.setup.clone());
            crate::account::add_with_password(
                store,
                &request.address,
                Some(&request.setup),
                false,
                false,
                now(),
                crate::account::Credentials {
                    password: request.password.as_ref(),
                    saved: &OAuthRegistry::default(),
                    secrets: &adding.secrets,
                    on_url,
                },
            )
        }),
        client: Arc::new(move |_| client),
        browse: Arc::new(move |url| {
            browsing.browsed.lock().unwrap().push(url.to_owned());
            Ok(())
        }),
    }
}

/// The `on_url` for an add that must never reach a browser sign-in.
fn no_browser(url: &str) {
    panic!("a browser sign-in was started: {url}");
}

fn offered(stage: Stage) -> Offer {
    match stage {
        Stage::Found(offer) => offer,
        other => panic!("nothing was found: {other:?}"),
    }
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (SqliteStore::in_memory(dir.path()).unwrap(), dir)
}

#[test]
fn an_unknown_domain_is_looked_up_once_and_shown_row_by_row() {
    let fake = Arc::new(Fake::default());
    let seams = seams(&fake, Ok(found("ada@example.test")), false);
    let offer = offered(flow::look("  Ada@Example.test ", &seams, now()));
    assert_eq!(fake.looked(), 1);
    assert_eq!(offer.address, "ada@example.test");
    assert_eq!(offer.source, "from the Thunderbird ISPDB");
    assert_eq!(offer.sign_in, SignIn::Password);
    let what: Vec<&str> = offer.rows.iter().map(|(what, _)| what.as_str()).collect();
    assert_eq!(what, ["incoming", "outgoing", "sign-in"]);
    assert!(
        offer.rows[0].1.contains("IMAP imap.example.test:993"),
        "{:?}",
        offer.rows
    );
    assert!(
        offer.rows[1].1.contains("SMTP smtp.example.test:465"),
        "{:?}",
        offer.rows
    );
    assert_eq!(fake.added(), 0, "looking added something");
}

#[test]
fn a_known_domain_is_not_looked_up_but_is_still_shown() {
    let fake = Arc::new(Fake::default());
    let seams = seams(&fake, Err("unreachable".to_owned()), false);
    let offer = offered(flow::look("ada@gmail.com", &seams, now()));
    assert_eq!(fake.looked(), 0, "the built-in table needs no lookup");
    assert!(
        fake.searched.lock().unwrap().is_empty(),
        "nor a JMAP search"
    );
    assert_eq!(offer.source, "from the built-in table");
    assert_eq!(
        offer.sign_in,
        SignIn::OAuth {
            issuer: OAuthIssuer::Google,
            client: Client::Missing,
        }
    );
    assert!(flow::before_looking("ada@gmail.com", now()).contains("nothing is looked up"));
}

#[test]
fn a_lookup_error_is_said_as_it_came() {
    let fake = Arc::new(Fake::default());
    let why = "could not find servers for ada@nowhere.test: no autoconfig, no SRV, no MX";
    let seams = seams(&fake, Err(why.to_owned()), false);
    let Stage::Missed(said) = flow::look("ada@nowhere.test", &seams, now()) else {
        panic!("something was found");
    };
    assert!(said.starts_with(why), "{said}");
    assert!(
        said.contains(&format!(
            "No JMAP server answered at https://nowhere.test/.well-known/jmap either: {NO_JMAP}"
        )),
        "{said}"
    );
    assert!(said.contains("enter it by hand"), "{said}");
}

const SESSION: &str = "https://jmap.example.test/session";
const TOKEN: &str = "fmu1-tok3n-0f-4417";

#[test]
fn a_domain_with_both_offers_what_its_autoconfig_named_and_jmap_beside_it() {
    let fake = Arc::new(Fake::default());
    let seams = with_jmap(
        &fake,
        Ok(found("ada@example.test")),
        Ok(SESSION.to_owned()),
        false,
    );
    let stage = flow::look("ada@example.test", &seams, now());
    // Only the domain's well-known URL went to the JMAP search, as the lookup got only a lookup.
    assert_eq!(
        *fake.searched.lock().unwrap(),
        ["https://example.test/.well-known/jmap"]
    );
    assert_eq!(fake.looked(), 1);
    let offer = offered(stage.clone());
    assert_eq!(
        offer.way(),
        "IMAP and SMTP",
        "the autoconfig's way is first"
    );
    assert!(matches!(offer.setup, crate::cli::Setup::Discovered(_)));
    assert_eq!(
        flow::ways(&offer),
        [
            (false, "IMAP and SMTP".to_owned()),
            (true, "JMAP".to_owned())
        ]
    );
    let said = flow::both(&offer).unwrap();
    assert!(
        said.contains(
            "JMAP at https://jmap.example.test/session, found at \
             https://example.test/.well-known/jmap"
        ),
        "{said}"
    );
    assert!(
        said.contains("IMAP and SMTP, found from the Thunderbird ISPDB"),
        "{said}"
    );

    let jmap = offered(flow::pick_way(&stage, true).unwrap());
    assert_eq!(jmap.jmap_auth(), Some(mail_domain::HttpAuth::Basic));
    assert_eq!(jmap.rows[0].1, format!("JMAP at {SESSION}"));
    assert_eq!(jmap.source, "at https://example.test/.well-known/jmap");
    assert_eq!(flow::ways(&jmap), flow::ways(&offer), "the order holds");
    assert_eq!(flow::pick_way(&Stage::Found(jmap.clone()), true), None);
    let back = offered(flow::pick_way(&Stage::Found(jmap), false).unwrap());
    assert_eq!(back, offer);
    assert_eq!(fake.added(), 0);
}

#[test]
fn jmap_found_alone_is_offered_and_added_with_its_session() {
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let seams = with_jmap(
        &fake,
        Err("no autoconfig".to_owned()),
        Ok(SESSION.to_owned()),
        false,
    );
    let offer = offered(flow::look("ada@example.test", &seams, now()));
    assert!(offer.other.is_none());
    assert_eq!(flow::both(&offer), None);
    assert_eq!(offer.sign_in, SignIn::Password);
    let stage = flow::confirm(
        &store,
        offer,
        Password::new("s3cret-pass".to_owned()),
        seams.add.as_ref(),
        &no_browser,
    );
    let Stage::Added { said, account } = &stage else {
        panic!("not added: {stage:?}");
    };
    assert_eq!(
        *fake.setups.lock().unwrap(),
        [crate::cli::Setup::Jmap {
            session: Some(SESSION.to_owned()),
            login: None,
            auth: mail_domain::HttpAuth::Basic,
        }]
    );
    assert_eq!(fake.kept(account.unwrap()).as_deref(), Some("s3cret-pass"));
    assert!(
        said.iter()
            .any(|line| line.contains("The password is in the system keyring")),
        "{said:?}"
    );
}

#[test]
fn a_session_typed_by_hand_with_a_token_adds_with_bearer() {
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let seams = seams(&fake, Err("unused".to_owned()), false);
    let hand = flow::Hand {
        session: format!("  {SESSION} "),
        auth: mail_domain::HttpAuth::Bearer,
    };
    let offer = flow::by_hand("Ada@Example.test", &hand).unwrap();
    assert!(offer.token());
    assert_eq!(offer.source, "by hand");
    assert_eq!(offer.rows[2].1, "an API token, sent as a bearer token");
    let empty = flow::confirm(
        &store,
        offer.clone(),
        Password::default(),
        seams.add.as_ref(),
        &no_browser,
    );
    assert_eq!(
        empty,
        Stage::Refused(offer.clone(), "Type the token first.".to_owned())
    );
    let stage = flow::confirm(
        &store,
        offer,
        Password::new(TOKEN.to_owned()),
        seams.add.as_ref(),
        &no_browser,
    );
    let Stage::Added { said, account } = &stage else {
        panic!("not added: {stage:?}");
    };
    assert_eq!(
        *fake.setups.lock().unwrap(),
        [crate::cli::Setup::Jmap {
            session: Some(SESSION.to_owned()),
            login: None,
            auth: mail_domain::HttpAuth::Bearer,
        }]
    );
    assert_eq!(fake.kept(account.unwrap()).as_deref(), Some(TOKEN));
    assert!(
        said.contains(&"The token is in the system keyring.".to_owned()),
        "{said:?}"
    );
    assert!(!format!("{stage:?}").contains(TOKEN), "{stage:?}");
    assert_eq!(fake.looked(), 0, "a typed server is not looked up");
    assert!(fake.searched.lock().unwrap().is_empty());
}

#[test]
fn a_session_url_by_hand_must_be_https_and_says_why() {
    let hand = |session: &str| flow::Hand {
        session: session.to_owned(),
        auth: mail_domain::HttpAuth::Basic,
    };
    assert_eq!(
        flow::by_hand("ada@example.test", &hand("")).unwrap_err(),
        "Type the JMAP session URL."
    );
    for bad in [
        "http://jmap.example.test/session",
        "jmap.example.test",
        "https://",
    ] {
        let why = flow::by_hand("ada@example.test", &hand(bad)).unwrap_err();
        assert!(why.contains("https://"), "{bad}: {why}");
    }
    assert!(flow::by_hand("ada", &hand(SESSION)).is_err());
    assert!(flow::by_hand("ada@example.test", &hand(SESSION)).is_ok());
}

#[test]
fn typing_a_server_in_starts_from_what_was_found_and_goes_back_to_looking() {
    let fake = Arc::new(Fake::default());
    let seams = with_jmap(
        &fake,
        Ok(found("ada@example.test")),
        Ok(SESSION.to_owned()),
        false,
    );
    let found = flow::look("ada@example.test", &seams, now());
    let Stage::ByHand(hand) = flow::alternate(&found) else {
        panic!("not typing a server in");
    };
    assert_eq!(hand.session, SESSION);
    let bearer = flow::pick_auth(&Stage::ByHand(hand.clone()), mail_domain::HttpAuth::Bearer);
    assert_eq!(
        bearer,
        Some(Stage::ByHand(flow::Hand {
            auth: mail_domain::HttpAuth::Bearer,
            ..hand.clone()
        }))
    );
    assert_eq!(
        flow::pick_auth(&Stage::ByHand(hand.clone()), mail_domain::HttpAuth::Basic),
        None
    );
    assert_eq!(flow::alternate(&Stage::ByHand(hand)), Stage::Blank);
    assert_eq!(
        flow::alternate(&Stage::Blank),
        Stage::ByHand(flow::Hand::blank())
    );
    // An IMAP offer has no JMAP sign-in to change.
    assert_eq!(flow::pick_auth(&found, mail_domain::HttpAuth::Bearer), None);
}

#[test]
fn what_is_not_an_address_is_never_looked_up() {
    let fake = Arc::new(Fake::default());
    let seams = seams(&fake, Ok(found("x@example.test")), false);
    for typed in ["", "ada", "ada@", "@example.test", "ada@localhost"] {
        assert!(
            matches!(flow::look(typed, &seams, now()), Stage::Missed(_)),
            "{typed:?}"
        );
    }
    assert_eq!(fake.looked(), 0);
    assert!(fake.searched.lock().unwrap().is_empty());
}

#[test]
fn the_sheet_says_only_the_domain_leaves_before_anything_does() {
    let said = flow::before_looking("ada@example.test", now());
    assert!(said.contains("only the domain, example.test"), "{said}");
    assert!(!said.contains("ada@"), "{said}");
}

#[test]
fn using_the_offer_hands_the_password_to_the_add_and_it_lands_in_the_keyring_fake() {
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let seams = seams(&fake, Ok(found("ada@example.test")), false);
    let offer = offered(flow::look("ada@example.test", &seams, now()));
    let stage = flow::confirm(
        &store,
        offer,
        Password::new("s3cret-pass".to_owned()),
        seams.add.as_ref(),
        &no_browser,
    );
    let Stage::Added { said, account } = &stage else {
        panic!("not added: {stage:?}");
    };
    assert_eq!(fake.added(), 1);
    assert_eq!(*fake.handed.lock().unwrap(), ["s3cret-pass"]);
    assert_eq!(fake.kept(account.unwrap()).as_deref(), Some("s3cret-pass"));
    assert_eq!(said[0], "Added ada@example.test.");
    assert!(
        said.iter().any(|line| line.contains("system keyring")),
        "{said:?}"
    );
    assert!(!format!("{stage:?}").contains("s3cret"), "{stage:?}");
}

#[test]
fn nothing_is_added_without_a_password_or_a_client_id() {
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let seams_pw = seams(&fake, Ok(found("ada@example.test")), false);
    let offer = offered(flow::look("ada@example.test", &seams_pw, now()));
    let stage = flow::confirm(
        &store,
        offer,
        Password::default(),
        seams_pw.add.as_ref(),
        &no_browser,
    );
    assert!(matches!(stage, Stage::Refused(..)), "{stage:?}");

    let gmail = offered(flow::look("ada@gmail.com", &seams_pw, now()));
    let stage = flow::confirm(
        &store,
        gmail,
        Password::default(),
        seams_pw.add.as_ref(),
        &no_browser,
    );
    let Stage::Refused(_, why) = stage else {
        panic!("added without a client id");
    };
    assert!(why.contains("MAILO_OAUTH_CLIENT_ID"), "{why}");
    assert_eq!(fake.added(), 0);
    assert!(crate::ui::data::account_rows(&store).is_empty());
}

#[test]
fn a_refused_add_says_so_and_keeps_the_offer() {
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let seams = seams(&fake, Ok(found("ada@example.test")), false);
    let offer = offered(flow::look("ada@example.test", &seams, now()));
    let refuse: &flow::Add =
        &|_, _, _| Err("cannot save the password: the keyring is locked".to_owned());
    let stage = flow::confirm(
        &store,
        offer.clone(),
        Password::new("pw".to_owned()),
        refuse,
        &no_browser,
    );
    assert_eq!(
        stage,
        Stage::Refused(
            offer,
            "Not added: cannot save the password: the keyring is locked".to_owned()
        )
    );
}

#[test]
fn what_add_printed_is_said_in_words() {
    let said = flow::in_words(
        "added ada@example.test as 6f1c\n\
         password stored in the keyring for login \"ada\"\n\
         \n\
         warning: this server wants an app password\n",
    );
    assert_eq!(
        said,
        [
            "Added ada@example.test.",
            "The password is in the system keyring, for the login ada.",
            "Warning: this server wants an app password",
        ]
    );
    let signed =
        flow::in_words("updated ada@gmail.com as 1\nsigned in; token stored in the keyring");
    assert_eq!(
        signed,
        [
            "ada@gmail.com was already here; its settings are updated.",
            "Signed in. The sign-in is in the system keyring.",
        ]
    );
}

#[test]
fn a_new_account_joins_a_scoped_space_and_not_an_open_one() {
    let account = AccountId::generate();
    let mut open = Space::default();
    assert!(!flow::widen(&mut open, account));
    assert_eq!(open.scope, Scope::All);

    let other = AccountId::generate();
    let mut scoped = Space {
        scope: Scope::Accounts(vec![other]),
        ..Space::default()
    };
    assert!(flow::widen(&mut scoped, account));
    assert!(!flow::widen(&mut scoped, account), "added twice");
    assert_eq!(scoped.scope, Scope::Accounts(vec![other, account]));
}

const SIGN_IN: &str = "https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz";

#[test]
fn the_sign_in_address_goes_from_the_add_to_whoever_is_waiting() {
    let (store, _dir) = store();
    let fake = Arc::new(Fake::default());
    let seams = seams(&fake, Err("unused".to_owned()), true);
    let gmail = offered(flow::look("ada@gmail.com", &seams, now()));
    let signs_in: &flow::Add = &|_, _, on_url| {
        on_url(SIGN_IN);
        Err("the sign-in was abandoned".to_owned())
    };
    let heard = Mutex::new(Vec::new());
    let stage = flow::confirm(&store, gmail, Password::default(), signs_in, &|url| {
        heard.lock().unwrap().push(url.to_owned())
    });
    assert!(matches!(stage, Stage::Refused(..)), "{stage:?}");
    assert_eq!(*heard.lock().unwrap(), [SIGN_IN]);
    assert!(
        fake.browsed.lock().unwrap().is_empty(),
        "confirm itself opened a browser"
    );
}

#[test]
fn the_sheet_is_told_whether_a_browser_took_the_address() {
    let opens: &flow::Browse = &|_| Ok(());
    let fails: &flow::Browse = &|_| Err("no browser found".to_owned());
    let cases: &[(&str, &flow::Browse, flow::Opened)] = &[
        ("opened", opens, flow::Opened::Browser),
        (
            "failed",
            fails,
            flow::Opened::Not("no browser found".to_owned()),
        ),
    ];
    for (name, browse, opened) in cases {
        assert_eq!(
            flow::signing_in(SIGN_IN, browse),
            flow::SigningIn {
                url: SIGN_IN.to_owned(),
                opened: opened.clone(),
            },
            "{name}"
        );
    }
}

#[test]
fn the_real_browser_seam_refuses_in_tests() {
    let real = Seams::real();
    assert!((real.jmap)("https://example.test/.well-known/jmap").is_err());
    assert_eq!(
        flow::signing_in(SIGN_IN, real.browse.as_ref()).opened,
        flow::Opened::Not("no browser is opened in tests".to_owned())
    );
}
