//! The keys sheet as drawn: the user's keys first, and the two acts that cannot be undone each
//! asked again before anything happens.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_runtime::{MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};

use super::keys::{KeysSheet, ordered};
use super::short;
use super::tests::{own_key, seams_with, someone_elses};
use crate::ui::fixtures::{Seen, click, dispatching, rebuild_into, seeded};
use crate::view::Shell;

#[component]
fn Sheet() -> Element {
    let shell = use_signal(|| Shell {
        keys: Some(crate::view::KeysSheet),
        ..Shell::default()
    });
    rsx! { KeysSheet { shell } }
}

pub(super) fn sheet(store: &Arc<SqliteStore>, seams: super::Seams) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new(Sheet)
        .with_root_context(store.clone())
        .with_root_context(seams);
    let seen = rebuild_into(&mut dom);
    (dom, seen)
}

/// Let work on its blocking thread land and redraw.
pub(super) async fn settle(dom: &mut VirtualDom, seen: &mut Seen) {
    for _ in 0..30 {
        let quiet = std::time::Duration::from_millis(100);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(seen);
    }
}

fn key(fingerprint: u8, secret: SecretHeld) -> PgpKey {
    PgpKey {
        fingerprint: Fingerprint::V4([fingerprint; 20]),
        key_ids: vec![],
        user_ids: vec![format!("key {fingerprint}")],
        emails: vec![],
        key: vec![],
        source: KeySource::Imported,
        first_seen: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        trust: KeyTrust::Unverified,
        created: None,
        expires: None,
        secret,
    }
}

/// A store with a key of the user's own and bea's public key, and the keyring holding the first.
fn two_keys() -> (
    Arc<SqliteStore>,
    tempfile::TempDir,
    Arc<MapSecrets>,
    PgpKey,
    PgpKey,
) {
    let (store, dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    let bea = someone_elses("bea@example.test", 71);
    let armored = bea.public().armored().unwrap();
    let theirs = crate::pgp::keys::import(
        &store,
        secrets.as_ref(),
        armored.as_bytes(),
        chrono::Utc::now(),
    )
    .unwrap()
    .remove(0)
    .key;
    let mine = own_key(&store, &secrets);
    (store, dir, secrets, mine, theirs)
}

fn held(secrets: &MapSecrets, key: &PgpKey) -> bool {
    secrets
        .get(&SecretKey {
            account: crate::ui::fixtures::ACCOUNT,
            purpose: SecretPurpose::OpenPgp(key.fingerprint),
        })
        .is_ok()
}

#[test]
fn the_users_own_keys_come_first() {
    let listed = ordered(vec![
        key(1, SecretHeld::Absent),
        key(2, SecretHeld::Held),
        key(3, SecretHeld::Absent),
        key(4, SecretHeld::Held),
    ]);
    let order: Vec<u8> = listed.iter().map(|k| k.fingerprint.as_bytes()[0]).collect();
    assert_eq!(order, [2, 4, 1, 3]);
}

#[tokio::test]
async fn the_sheet_lists_own_keys_first_with_what_is_known_of_each() {
    let (store, _dir, secrets, mine, theirs) = two_keys();
    let (dom, _) = sheet(&store, seams_with(secrets));
    let page = dioxus_ssr::render(&dom);
    let at_mine = page.find(&short(mine.fingerprint)).unwrap();
    let at_theirs = page.find(&short(theirs.fingerprint)).unwrap();
    assert!(at_mine < at_theirs, "{page}");
    assert!(page.contains("secret key held here"), "{page}");
    assert!(page.contains("keys-row mine"), "{page}");
    assert!(page.contains("Them &#60;bea@example.test&#62;"), "{page}");
    // Me has a key, so nothing is offered to make one.
    assert!(!page.contains("Make a key for"), "{page}");
}

#[tokio::test]
async fn deleting_a_key_with_its_secret_is_asked_again_first() {
    let (store, _dir, secrets, mine, _) = two_keys();
    let (mut dom, seen) = sheet(&store, seams_with(secrets.clone()));
    let id = short(mine.fingerprint);

    let mut asked = click(&mut dom, seen.one("aria-label", &format!("Delete {id}")));
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("cannot be recovered"), "{page}");
    assert!(
        store.pgp_key(mine.fingerprint).unwrap().is_some(),
        "deleted before the answer"
    );
    assert!(held(&secrets, &mine));

    // Cancel keeps it.
    click(
        &mut dom,
        asked.one("aria-label", "Cancel: Delete the key and its secret"),
    );
    assert!(!dioxus_ssr::render(&dom).contains("cannot be recovered"));
    assert!(held(&secrets, &mine));

    asked = click(&mut dom, seen.one("aria-label", &format!("Delete {id}")));
    let mut done = click(
        &mut dom,
        asked.one("aria-label", "Delete the key and its secret"),
    );
    settle(&mut dom, &mut done).await;
    assert!(store.pgp_key(mine.fingerprint).unwrap().is_none());
    assert!(
        !held(&secrets, &mine),
        "the secret half is still in the keyring"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("Deleted the key of"), "{page}");
    // With its key gone, the address may have one made again.
    assert!(page.contains("Make a key for me@example.test"), "{page}");
}

#[tokio::test]
async fn a_key_without_a_secret_is_deleted_at_once() {
    let (store, _dir, secrets, _, theirs) = two_keys();
    let (mut dom, seen) = sheet(&store, seams_with(secrets));
    let mut done = click(
        &mut dom,
        seen.one(
            "aria-label",
            &format!("Delete {}", short(theirs.fingerprint)),
        ),
    );
    settle(&mut dom, &mut done).await;
    assert!(store.pgp_key(theirs.fingerprint).unwrap().is_none());
}

#[tokio::test]
async fn exporting_the_secret_key_is_asked_again_and_only_then_written() {
    let (store, dir, secrets, mine, _) = two_keys();
    let calls = Arc::new(AtomicUsize::new(0));
    let out: PathBuf = dir.path().join("secret.asc");
    let mut seams = seams_with(secrets);
    seams.save = {
        let calls = calls.clone();
        let out = out.clone();
        Arc::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(out.clone())
        })
    };
    let (mut dom, seen) = sheet(&store, seams);
    let id = short(mine.fingerprint);

    let asked = click(
        &mut dom,
        seen.one("aria-label", &format!("Export the secret key {id}")),
    );
    dom.render_immediate(&mut NoOpMutations);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Anyone who has that file can read your encrypted mail"),
        "{page}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "asked where to save before the answer"
    );
    assert!(!out.exists());

    let mut done = click(&mut dom, asked.one("aria-label", "Save the secret key…"));
    settle(&mut dom, &mut done).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(written.contains("PRIVATE KEY"), "{written}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "a secret key readable by others");
    }
    assert!(dioxus_ssr::render(&dom).contains("Keep that file offline"));
}

#[tokio::test]
async fn generate_verify_and_import_go_through_the_keyring_handed_in() {
    let (store, dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    let bea = someone_elses("bea@example.test", 72);
    let file = dir.path().join("bea.asc");
    std::fs::write(&file, bea.public().armored().unwrap()).unwrap();
    let mut seams = seams_with(secrets.clone());
    seams.pick = Arc::new(move || Some(file.clone()));
    let (mut dom, seen) = sheet(&store, seams);

    let mut made = click(
        &mut dom,
        seen.one("aria-label", "Make a key for me@example.test"),
    );
    settle(&mut dom, &mut made).await;
    let mine = crate::pgp::own_key(&store, "me@example.test")
        .unwrap()
        .unwrap();
    assert!(held(&secrets, &mine));

    let mut imported = click(&mut dom, seen.one("aria-label", "Import from a file…"));
    settle(&mut dom, &mut imported).await;
    let theirs = store.pgp_key(bea.fingerprint()).unwrap().unwrap();
    assert!(dioxus_ssr::render(&dom).contains("Imported Them"));

    let all = seen.merge(made).merge(imported);
    let mut verified = click(
        &mut dom,
        all.one(
            "aria-label",
            &format!("Mark {} verified", short(theirs.fingerprint)),
        ),
    );
    settle(&mut dom, &mut verified).await;
    assert_eq!(
        store.pgp_key(bea.fingerprint()).unwrap().unwrap().trust,
        KeyTrust::Verified
    );
}

#[tokio::test]
async fn every_class_on_the_sheet_is_styled() {
    let (store, _dir, secrets, mine, _) = two_keys();
    let (mut dom, seen) = sheet(&store, seams_with(secrets));
    click(
        &mut dom,
        seen.one("aria-label", &format!("Delete {}", short(mine.fingerprint))),
    );
    let page = dioxus_ssr::render(&dom);
    assert!(page.contains("keys-confirm"), "{page}");
    let missing =
        crate::ui::style::tests::unstyled_classes(&page, &crate::ui::style::tests::full_css());
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}
