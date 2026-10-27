//! The certificates Settings lists, as drawn on an account's page then on Other Keys: the user's
//! own certificates first, an identity file's password asked for in a sheet and never drawn,
//! deleting a private key asked again in an alert, trust given and taken back — and the OpenPGP
//! half's dates.

use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_core::SigningStore;
use mail_core::testing::MapSigningStore;
use mail_core::{SqliteStore, Store};
use mail_domain::signing::{SigningKeyId, SigningKeyRef};
use mail_domain::*;

use super::certs::ordered;
use super::key_row::lifetime;
use super::keys::{KeysPage, Whose};
use super::keys_tests::{alert_button, pick, settle};
use super::smime_tests::{PASSWORD, identity_file, trust_root, with_identity};
use super::tests::{own_key, seams_with};
use super::{Seams, cert_short, short};
use crate::ui::fixtures::smime_support::pki;
use crate::ui::fixtures::{
    Seen, acct_account, click, dispatching, drain_seen, rebuild_into, seeded, type_into,
};

/// The page inside a quire root, as the Settings window has it.
#[component]
fn Page() -> Element {
    rsx! {
        ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window,
            ds::prelude::Form {
                KeysPage { whose: Whose::Account(super::tests::ME.to_owned()) }
                KeysPage { whose: Whose::Others }
            }
        }
    }
}

fn sheet(store: &Arc<SqliteStore>, seams: Seams) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new_with_props(Page, ())
        .with_root_context(crate::ui::fixtures::client(store))
        .with_root_context(seams);
    let seen = rebuild_into(&mut dom);
    let seen = seen.merge(drain_seen(&mut dom));
    (dom, seen)
}

fn markup(dom: &VirtualDom) -> String {
    dioxus_ssr::render(dom).replace("&#39;", "'")
}

fn cert(fingerprint: u8, secret: SecretHeld) -> SmimeCert {
    let at = Utc::now();
    SmimeCert {
        fingerprint: CertFingerprint([fingerprint; 32]),
        subject: format!("CN=Cert {fingerprint}"),
        issuer: "CN=CA".to_owned(),
        serial: "01".to_owned(),
        emails: vec![],
        not_before: at,
        not_after: at,
        der: vec![],
        chain: vec![],
        source: CertSource::Imported,
        first_seen: at,
        last_seen: at,
        trust: KeyTrust::Unverified,
        secret,
    }
}

fn holds(secrets: &MapSigningStore, cert: &SmimeCert) -> bool {
    secrets
        .get(&SigningKeyRef {
            account: acct_account(),
            key: SigningKeyId::Smime(cert.fingerprint),
        })
        .is_ok()
}

#[test]
fn the_users_own_certificates_come_first() {
    let listed = ordered(vec![
        cert(1, SecretHeld::Absent),
        cert(2, SecretHeld::Held),
        cert(3, SecretHeld::Absent),
        cert(4, SecretHeld::Held),
    ]);
    let order: Vec<u8> = listed.iter().map(|c| c.fingerprint.0[0]).collect();
    assert_eq!(order, [2, 4, 1, 3]);
}

#[tokio::test]
async fn certificates_are_listed_own_first_with_what_is_known_of_each() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    trust_root(&store, &secrets);
    let mine = with_identity(&store, &secrets);
    let (dom, _) = sheet(&store, seams_with(secrets));
    let page = markup(&dom);
    // Mine, then the authorities its file carried, then the root imported first of all.
    let at_mine = page.find(&cert_short(mine.fingerprint)).unwrap();
    let at_root = page
        .find(&cert_short(pki().root.cert.fingerprint()))
        .unwrap();
    assert!(at_mine < at_root, "{page}");
    assert!(page.contains("CN=Me"), "{page}");
    assert!(
        page.contains(
            "me@example.test · valid 2026-01-01 to 2028-01-01 · issued by CN=Example Test Mail CA"
        ),
        "{page}"
    );
    assert!(
        page.contains("your identity · not trusted by you · private key held here"),
        "{page}"
    );
    assert!(page.contains("imported · trusted by you"), "{page}");
}

#[tokio::test]
async fn an_identity_files_password_is_asked_in_a_sheet_and_never_drawn() {
    let (store, dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let file = dir.path().join("me.p12");
    std::fs::write(&file, identity_file()).unwrap();
    let mut seams = seams_with(secrets.clone());
    seams.pick = Arc::new(move || Some(file.clone()));
    let (mut dom, seen) = sheet(&store, seams);

    let mut asked = click(
        &mut dom,
        seen.all("aria-label", "Import a certificate or identity…")[0],
    );
    settle(&mut dom, &mut asked).await;
    let prompt = "me.p12 is an identity file. Type its password to import it.";
    assert!(markup(&dom).contains(prompt), "{}", markup(&dom));
    assert!(store.smime_certs().unwrap().is_empty(), "imported unasked");

    let typed = type_into(
        &mut dom,
        asked.one("aria-label", &format!("Password: {prompt}")),
        PASSWORD,
    );
    assert!(
        !markup(&dom).contains(PASSWORD),
        "the password is in the markup"
    );
    let mut done = click(&mut dom, asked.merge(typed).one("aria-label", "Import"));
    settle(&mut dom, &mut done).await;
    let page = markup(&dom);
    assert!(
        !store.smime_certs().unwrap().is_empty(),
        "the identity was not imported"
    );
    assert!(!page.contains(PASSWORD), "the password is in the markup");
    assert!(!page.contains(prompt), "still asking: {page}");
    let own = mail_core::smime::own_cert(&store, "me@example.test", Utc::now())
        .unwrap()
        .unwrap();
    assert!(holds(&secrets, &own));
}

#[tokio::test]
async fn deleting_a_certificate_with_its_private_key_is_asked_again_first() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let mine = with_identity(&store, &secrets);
    let (mut dom, seen) = sheet(&store, seams_with(secrets.clone()));
    let id = cert_short(mine.fingerprint);

    let asked = pick(&mut dom, &seen, &id, crate::ui::copy::menus::DELETE).await;
    assert!(markup(&dom).contains("private key is deleted too"));
    assert!(store.smime_cert(mine.fingerprint).unwrap().is_some());
    assert!(holds(&secrets, &mine));
    // Cancel, the alert's default, keeps it.
    click(&mut dom, alert_button(&asked, Some("return")));
    let _ = drain_seen(&mut dom);
    assert!(!markup(&dom).contains("private key is deleted too"));

    let asked = pick(&mut dom, &seen, &id, crate::ui::copy::menus::DELETE).await;
    let mut done = click(&mut dom, alert_button(&asked, None));
    settle(&mut dom, &mut done).await;
    assert!(store.smime_cert(mine.fingerprint).unwrap().is_none());
    assert!(
        !holds(&secrets, &mine),
        "the private key is still in the keyring"
    );
}

#[tokio::test]
async fn a_certificate_is_trusted_and_the_trust_taken_back() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let root = pki().root.cert.fingerprint();
    mail_core::smime::certs::import(
        &store,
        secrets.as_ref(),
        pki().root.cert.pem().as_bytes(),
        &|| None,
        Utc::now(),
    )
    .unwrap();
    let (mut dom, seen) = sheet(&store, seams_with(secrets));
    let id = cert_short(root);
    let trust = |store: &SqliteStore| store.smime_cert(root).unwrap().unwrap().trust;

    let mut trusted = pick(&mut dom, &seen, &id, crate::ui::copy::menus::TRUST).await;
    settle(&mut dom, &mut trusted).await;
    assert_eq!(trust(&store), KeyTrust::Verified);
    assert!(markup(&dom).contains("trusted by you"));
    let mut untrusted = pick(&mut dom, &seen, &id, crate::ui::copy::menus::DONT_TRUST).await;
    settle(&mut dom, &mut untrusted).await;
    assert_eq!(trust(&store), KeyTrust::Unverified);
}

#[test]
fn a_keys_dates_are_said_as_far_as_they_are_known() {
    let now = Utc::now();
    let key = |created: Option<i64>, expires: Option<i64>| PgpKey {
        fingerprint: Fingerprint::V4([1; 20]),
        key_ids: vec![],
        user_ids: vec![],
        emails: vec![],
        key: vec![],
        source: KeySource::Imported,
        first_seen: now,
        last_seen: now,
        trust: KeyTrust::Unverified,
        created: created.map(|days| now + TimeDelta::days(days)),
        expires: expires.map(|days| now + TimeDelta::days(days)),
        secret: SecretHeld::Absent,
    };
    let day = |days: i64| (now + TimeDelta::days(days)).format("%Y-%m-%d").to_string();
    let cases = [
        (
            key(Some(-30), None),
            vec![
                format!("created {}", day(-30)),
                "does not expire".to_owned(),
            ],
        ),
        (
            key(Some(-30), Some(700)),
            vec![
                format!("created {}", day(-30)),
                format!("expires {}", day(700)),
            ],
        ),
        (
            key(Some(-900), Some(-10)),
            vec![
                format!("created {}", day(-900)),
                format!("expired {}", day(-10)),
            ],
        ),
        // Dates never read from the key: nothing is claimed.
        (key(None, None), vec![]),
    ];
    for (given, want) in cases {
        assert_eq!(
            lifetime(&given, now),
            want,
            "{:?} {:?}",
            given.created,
            given.expires
        );
    }
}

#[tokio::test]
async fn the_sheet_says_when_a_key_was_made_and_until_when_it_holds() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let key = own_key(&store, &secrets);
    let stored = store.pgp_key(key.fingerprint).unwrap().unwrap();
    assert!(stored.created.is_some(), "a key made here knows when");
    let (dom, _) = sheet(&store, seams_with(secrets));
    let page = markup(&dom);
    assert!(page.contains(&short(key.fingerprint)), "{page}");
    for said in lifetime(&stored, Utc::now()) {
        assert!(page.contains(&said), "{said} missing: {page}");
    }
}

/// The certificates half asking to import, saying a change, and asking whether to delete, for
/// the style lint in `style::tests`.
pub(in crate::ui) async fn drawn_states() -> String {
    let (store, dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let mine = with_identity(&store, &secrets);
    trust_root(&store, &secrets);
    let file = dir.path().join("me.p12");
    std::fs::write(&file, identity_file()).unwrap();
    let mut seams = seams_with(secrets);
    seams.pick = Arc::new(move || Some(file.clone()));
    let (mut dom, seen) = sheet(&store, seams);
    let mut asked = click(
        &mut dom,
        seen.all("aria-label", "Import a certificate or identity…")[0],
    );
    settle(&mut dom, &mut asked).await;
    let mut page = markup(&dom);
    assert!(page.contains("keys-ask"), "{page}");
    let root = cert_short(pki().root.cert.fingerprint());
    let mut untrusted = pick(&mut dom, &seen, &root, crate::ui::copy::menus::DONT_TRUST).await;
    settle(&mut dom, &mut untrusted).await;
    page += &markup(&dom);
    // What a change came to is said in quire's banner.
    assert!(page.contains("ds-inline-banner"), "{page}");
    pick(
        &mut dom,
        &seen,
        &cert_short(mine.fingerprint),
        crate::ui::copy::menus::DELETE,
    )
    .await;
    // The question is quire's alert, drawn in the root's overlay the render after it asks.
    crate::ui::fixtures::drain(&mut dom);
    page += &markup(&dom);
    assert!(
        page.contains("ds-alert") && page.contains("ds-section-header-title"),
        "{page}"
    );
    page
}
