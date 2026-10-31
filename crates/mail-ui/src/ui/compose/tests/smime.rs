//! The composer's one protection control, and S/MIME from it: the control can never ask for
//! both protections, what `smime::check` says stands in the way is said in the warning bar, and
//! a locked OpenPGP key is known by the send's typed error.

use mail_core::Store;
use mail_core::testing::MapSigningStore;
use rand::SeedableRng;

use super::super::protection::{Protection, items, label, pick};
use super::super::seal::SealBar;
use super::*;
use crate::ui::fixtures::smime_support::{Person, identity, pki, rng};
use crate::ui::fixtures::{Seen, acct_account, click, rebuild_into, seeded};
use mail_core::SqliteStore;
use std::sync::Arc;

thread_local! {
    /// The Settings windows the page last drawn asked for: the composer's bar sends a key to
    /// make to the sending account's page in Settings.
    static ASKED: std::cell::RefCell<Arc<crate::ui::settings_window::tests::Asked>> =
        std::cell::RefCell::new(Arc::default());
}

const PASSWORD: &str = "p12 password";

/// Every choice the menu offers, by key.
fn keys() -> Vec<String> {
    items(Protection::None)
        .into_iter()
        .map(|item| item.key)
        .collect()
}

/// A draft to dana asking `openpgp` and `smime`, stored.
fn asking(store: &SqliteStore, openpgp: Option<Mode>, smime: Option<Mode>) -> Draft {
    let to = [Address {
        name: Some("Dana Whitfield".to_owned()),
        email: "dana@example.test".to_owned(),
    }];
    let mut draft = mail_core::compose::draft_new(
        store,
        acct_account(),
        &to,
        "Plans",
        "see you there",
        Utc::now(),
    )
    .unwrap();
    draft.openpgp = openpgp;
    draft.smime = smime;
    mail_core::compose::save(store, &draft).unwrap();
    draft
}

fn window(store: Arc<SqliteStore>, draft: Draft, secrets: Arc<MapSigningStore>) -> (Window, Seen) {
    crate::ui::fixtures::dispatching();
    let mut dom = VirtualDom::new_with_props(PageHarness, PageHarnessProps { draft })
        .with_root_context(crate::ui::fixtures::client(&store))
        .with_root_context(crate::ui::pgp::tests::seams_with(secrets))
        .with_root_context(crate::ui::settings_window::SettingsWindows(ASKED.with(
            |asked| {
                let fresh = Arc::new(crate::ui::settings_window::tests::Asked::default());
                *asked.borrow_mut() = fresh.clone();
                fresh
            },
        )));
    let seen = rebuild_into(&mut dom);
    let desk = DESK.with(Cell::get).unwrap();
    let shell = SHELL.with(Cell::get).unwrap();
    (Window { dom, desk, shell }, seen)
}

#[test]
fn the_one_control_can_never_ask_for_both_protections() {
    // A draft asking both, as only another client could leave one: the page reads one of them.
    let mut both = draft_of("");
    both.openpgp = Some(Mode::Sign);
    both.smime = Some(Mode::Encrypt);
    let read = Page::of(&both, Vec::new(), Vec::new());
    assert_eq!(read.protection, Protection::OpenPgp(Mode::Sign));
    let saved = read.apply_to(&both, at(1));
    assert_eq!((saved.openpgp, saved.smime), (Some(Mode::Sign), None));

    // From every choice, every pick, over a stored draft that asked both: never both.
    let keys = keys();
    for from in &keys {
        for to in &keys {
            let mut page = Page::of(&both, Vec::new(), Vec::new());
            pick(&mut page, from);
            pick(&mut page, to);
            let draft = page.apply_to(&both, at(2));
            assert!(
                draft.openpgp.is_none() || draft.smime.is_none(),
                "{from} then {to} asked for both: {:?} {:?}",
                draft.openpgp,
                draft.smime
            );
            // And the draft reads back as the choice made.
            assert_eq!(Protection::of(&draft), page.protection, "{from} then {to}");
        }
    }
    // Taking encryption off keeps the scheme's signature and never crosses schemes.
    for (given, want) in [
        (
            Protection::Smime(Mode::SignAndEncrypt),
            Protection::Smime(Mode::Sign),
        ),
        (Protection::Smime(Mode::Encrypt), Protection::None),
        (
            Protection::OpenPgp(Mode::SignAndEncrypt),
            Protection::OpenPgp(Mode::Sign),
        ),
        (
            Protection::OpenPgp(Mode::Sign),
            Protection::OpenPgp(Mode::Sign),
        ),
    ] {
        assert_eq!(given.without_encryption(), want);
    }
}

#[test]
fn picking_smime_clears_openpgp() {
    let mut base = draft_of("");
    base.openpgp = Some(Mode::SignAndEncrypt);
    let mut page = Page::of(&base, Vec::new(), Vec::new());
    assert_eq!(label(page.protection), "OpenPGP · Sign and Encrypt");
    page.seal_bar = SealBar::NoOwnKey("me@example.test".to_owned());
    pick(&mut page, "smime-sign");
    assert_eq!(page.protection, Protection::Smime(Mode::Sign));
    assert_eq!(
        page.seal_bar,
        SealBar::Clear,
        "a bar about the old choice stayed"
    );
    assert_eq!(label(page.protection), "S/MIME · Sign");
    let draft = page.apply_to(&base, at(1));
    assert_eq!((draft.openpgp, draft.smime), (None, Some(Mode::Sign)));
    // And back: OpenPGP clears S/MIME.
    pick(&mut page, "pgp-encrypt");
    let draft = page.apply_to(&draft, at(2));
    assert_eq!((draft.openpgp, draft.smime), (Some(Mode::Encrypt), None));
}

#[tokio::test]
async fn the_row_offers_both_schemes_in_one_menu_and_saves_the_choice() {
    let (store, _dir) = seeded();
    let draft = asking(&store, Some(Mode::Sign), None);
    let (mut window, seen) = window(store.clone(), draft.clone(), Arc::default());
    click(
        &mut window.dom,
        seen.one("aria-label", "Security: OpenPGP · Sign"),
    );
    let markup = window.render();
    assert!(
        markup.contains(">OpenPGP<") && markup.contains(">S/MIME<"),
        "{markup}"
    );
    assert!(
        !markup.contains("data-row=\"openpgp\""),
        "a second row: {markup}"
    );
    let mut page = window.page();
    window
        .dom
        .in_runtime(|| pick(&mut page.write(), "smime-sign-encrypt"));
    window.dom.in_runtime(|| {
        super::super::life::save(
            &crate::ui::fixtures::client(&store),
            &mut page.write(),
            Utc::now(),
        )
        .unwrap();
    });
    let stored = store.draft(draft.id).unwrap();
    assert_eq!(
        (stored.openpgp, stored.smime),
        (None, Some(Mode::SignAndEncrypt))
    );
}

#[tokio::test]
async fn what_smime_check_says_stands_in_the_way_is_said_in_the_bar() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let draft = asking(&store, None, Some(Mode::Encrypt));
    let (mut window, seen) = window(store.clone(), draft.clone(), secrets.clone());

    // No certificate of my own.
    let after = click(
        &mut window.dom,
        seen.one("aria-label", mail_ui_model::copy::controls::SEND),
    );
    let markup = window.render();
    assert!(
        markup.contains("No S/MIME certificate for me@example.test"),
        "{markup}"
    );
    assert!(markup.contains("class=\"ds-inline-banner\""), "{markup}");
    assert!(markup.contains("Send without S/MIME"), "{markup}");
    click(
        &mut window.dom,
        after.one("aria-label", "Import your certificate…"),
    );
    // Making a key or importing a certificate is the sending account's page's.
    assert_eq!(
        ASKED.with(|asked| asked.borrow().asks()),
        [Some(crate::ui::settings_window::SettingsAt::Account(
            acct_account()
        ))]
    );

    // With one, dana has none: S/MIME has no directory to ask, so only sending without
    // encryption is offered, and how her certificate would come is said.
    let me = identity(pki(), &Person::new("Me", &["me@example.test"], 20, 2001));
    let file = mail_crypto::smime::write_pkcs12(&me, PASSWORD, &mut rng(1)).unwrap();
    mail_core::smime::certs::import(
        &store,
        secrets.as_ref(),
        &file,
        &|| Some(PASSWORD.to_owned()),
        Utc::now(),
    )
    .unwrap();
    let after = click(
        &mut window.dom,
        seen.one("aria-label", mail_ui_model::copy::controls::SEND),
    );
    let markup = window.render();
    assert!(
        markup.contains("No S/MIME certificate for dana@example.test"),
        "{markup}"
    );
    assert!(!markup.contains("Look up keys"), "{markup}");
    let page = window.page();
    assert_eq!(
        window.dom.in_runtime(|| page.peek().seal_bar.clone()),
        SealBar::NoCertFor(vec!["dana@example.test".to_owned()])
    );

    click(
        &mut window.dom,
        after.one("aria-label", "Send without encryption"),
    );
    // S/MIME Encrypt without encryption is nothing: the plain send is queued at once.
    let stored = store.draft(draft.id).unwrap();
    assert_eq!((stored.openpgp, stored.smime), (None, None));
    assert_ne!(stored.state, SendState::Editing, "nothing was sent");
}

#[test]
fn a_locked_openpgp_key_is_known_by_the_sends_typed_error() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let key = mail_crypto::openpgp::generate(
        "Me <me@example.test>",
        Utc::now(),
        &mut rand::rngs::StdRng::seed_from_u64(83),
    )
    .unwrap()
    .with_passphrase("owl at dusk", &mut rand::rngs::StdRng::seed_from_u64(84))
    .unwrap();
    mail_core::pgp::keys::import(
        &store,
        secrets.as_ref(),
        key.armored().unwrap().as_bytes(),
        Utc::now(),
    )
    .unwrap();
    let draft = asking(&store, Some(Mode::Sign), None);

    // The data side's answer, typed: the key, whether or not a passphrase was asked for.
    let refused = super::super::life::queue(
        &crate::ui::fixtures::client(&store),
        secrets.as_ref(),
        &mail_core::pgp::no_passphrase,
        draft.id,
        mail_core::compose::Leaves::Now,
        Utc::now(),
    )
    .unwrap_err();
    assert_eq!(refused.locked(), Some(key.fingerprint()));

    // Given one, and the wrong one, it is still that key: the window says "try again".
    let wrong = |_: Fingerprint| Some("not the words".to_owned());
    let refused = super::super::life::queue(
        &crate::ui::fixtures::client(&store),
        secrets.as_ref(),
        &wrong,
        draft.id,
        mail_core::compose::Leaves::Now,
        Utc::now(),
    )
    .unwrap_err();
    assert_eq!(refused.locked(), Some(key.fingerprint()));
    assert!(queued_nothing(&store));
}

fn queued_nothing(store: &SqliteStore) -> bool {
    store
        .outbox_due(acct_account(), Utc::now() + chrono::TimeDelta::days(365))
        .unwrap()
        .is_empty()
}
