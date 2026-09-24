//! OpenPGP from the composer: the row writes the draft, what stands in the way is said in the
//! warning bar before anything is queued, and a locked key is asked for there.

use mail_runtime::MapSecrets;
use mail_store::Store;
use rand::SeedableRng;

use super::super::page::Float;
use super::super::protection::{Mode, Protection, items, pick};
use super::super::seal::SealBar;
use super::*;
use crate::ui::fixtures::{ACCOUNT, Seen, click, rebuild_into, seeded, type_into};

fn far() -> DateTime<Utc> {
    Utc::now() + chrono::TimeDelta::days(365)
}

fn queued(store: &SqliteStore) -> Vec<Vec<u8>> {
    store
        .outbox_due(ACCOUNT, far())
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry.op {
            ProtoOp::Submit { raw, .. } => store.blobs().get(&store.connection(), raw).ok(),
            _ => None,
        })
        .collect()
}

/// A draft to dana that asks for `openpgp`, stored.
fn asking(store: &SqliteStore, openpgp: OpenPgp) -> Draft {
    let to = [Address {
        name: Some("Dana Whitfield".to_owned()),
        email: "dana@example.test".to_owned(),
    }];
    let mut draft =
        crate::compose::draft_new(store, ACCOUNT, &to, "Plans", "see you there", Utc::now())
            .unwrap();
    draft.openpgp = openpgp;
    crate::compose::save(store, &draft).unwrap();
    draft
}

/// The page for `draft`, with `secrets` as the window's keyring.
fn window(store: Arc<SqliteStore>, draft: Draft, secrets: Arc<MapSecrets>) -> (Window, Seen) {
    window_with(store, draft, crate::ui::pgp::tests::seams_with(secrets))
}

fn window_with(
    store: Arc<SqliteStore>,
    draft: Draft,
    seams: crate::ui::pgp::Seams,
) -> (Window, Seen) {
    crate::ui::fixtures::dispatching();
    let mut dom = VirtualDom::new_with_props(PageHarness, PageHarnessProps { draft })
        .with_root_context(store)
        .with_root_context(seams);
    let seen = rebuild_into(&mut dom);
    let desk = DESK.with(Cell::get).unwrap();
    let shell = SHELL.with(Cell::get).unwrap();
    (Window { dom, desk, shell }, seen)
}

/// The element last drawn with `name` = `value`: a bar drawn again has new elements.
fn last(seen: &Seen, name: &str, value: &str) -> dioxus_core::ElementId {
    *seen
        .all(name, value)
        .last()
        .unwrap_or_else(|| panic!("nothing drawn with {name}={value:?}"))
}

/// Let the page's tasks run until `done` holds of its markup, or fifteen seconds pass.
async fn until(window: &mut Window, seen: &mut Seen, done: impl Fn(&str) -> bool) -> String {
    let give_up = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let page = dioxus_ssr::render(&window.dom);
        if done(&page) || tokio::time::Instant::now() > give_up {
            return page;
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            window.dom.wait_for_work(),
        )
        .await;
        window.dom.render_immediate(seen);
    }
}

#[test]
fn the_row_writes_the_drafts_openpgp() {
    let mut page = page_of("");
    assert_eq!(page.protection, Protection::None);
    let names: Vec<(Option<String>, String)> = items(page.protection)
        .into_iter()
        .map(|item| (item.group, item.name))
        .collect();
    let pgp = Some("OpenPGP".to_owned());
    let smime = Some("S/MIME".to_owned());
    assert_eq!(
        names,
        [
            (None, "None".to_owned()),
            (pgp.clone(), "Sign".to_owned()),
            (pgp.clone(), "Encrypt".to_owned()),
            (pgp, "Sign and encrypt".to_owned()),
            (smime.clone(), "Sign".to_owned()),
            (smime.clone(), "Encrypt".to_owned()),
            (smime, "Sign and encrypt".to_owned()),
        ]
    );
    page.float = Float::Protection;
    pick(&mut page, "pgp-sign-encrypt");
    assert_eq!(page.protection, Protection::OpenPgp(Mode::SignAndEncrypt));
    assert_eq!(page.float, Float::Closed);
    assert_eq!(page.saved, super::super::page::Saved::Dirty);
    assert_eq!(
        page.apply_to(&draft_of(""), at(1)).openpgp,
        OpenPgp::SignAndEncrypt
    );
    // Opened again, the page asks what the draft asks.
    let mut draft = draft_of("");
    draft.openpgp = OpenPgp::Sign;
    assert_eq!(
        Page::of(&draft, Vec::new(), Vec::new()).protection,
        Protection::OpenPgp(Mode::Sign)
    );
}

#[tokio::test]
async fn the_row_is_drawn_and_its_choice_saved() {
    let (store, _dir) = seeded();
    let draft = asking(&store, OpenPgp::None);
    let (mut window, seen) = window(store.clone(), draft.clone(), Arc::default());
    let markup = window.render();
    assert!(markup.contains("data-row=\"protection\""), "{markup}");
    click(&mut window.dom, seen.one("aria-label", "Protection: None"));
    let markup = window.render();
    assert!(markup.contains("Sign and encrypt"), "{markup}");
    let mut page = window.page();
    window
        .dom
        .in_runtime(|| pick(&mut page.write(), "pgp-sign"));
    window.dom.in_runtime(|| {
        super::super::life::save(&store, &mut page.write(), Utc::now()).unwrap();
    });
    assert_eq!(store.draft(draft.id).unwrap().openpgp, OpenPgp::Sign);
}

#[tokio::test]
async fn no_key_for_a_recipient_names_them_and_holds_the_send_until_a_choice() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    crate::pgp::keys::generate(&store, secrets.as_ref(), "me@example.test", Utc::now()).unwrap();
    let draft = asking(&store, OpenPgp::Encrypt);
    let (mut window, seen) = window(store.clone(), draft.clone(), secrets);

    let mut after = click(&mut window.dom, seen.one("aria-label", "Send"));
    let markup = window.render();
    assert!(
        markup
            .contains("No OpenPGP key for dana@example.test, so this cannot be encrypted to them."),
        "{markup}"
    );
    assert!(markup.contains("class=\"c-warn seal-warn\""), "{markup}");
    assert!(markup.contains("Look up keys"), "{markup}");
    assert!(queued(&store).is_empty(), "sent without a key");

    // Send again: still held.
    after = after.merge(click(&mut window.dom, seen.one("aria-label", "Send")));
    assert!(queued(&store).is_empty(), "the second press sent it");
    assert!(
        window
            .render()
            .contains("No OpenPGP key for dana@example.test")
    );

    // Looking the key up asks the seam, which here finds nothing, and says so.
    let mut looked = click(&mut window.dom, last(&after, "aria-label", "Look up keys"));
    let markup = until(&mut window, &mut looked, |page| {
        page.contains("Could not ask about dana@example.test")
    })
    .await;
    assert!(
        markup.contains("Could not ask about dana@example.test: no lookups in tests."),
        "{markup}"
    );
    // Still no key: the bar still names them.
    assert!(
        markup.contains("No OpenPGP key for dana@example.test"),
        "{markup}"
    );
    assert!(queued(&store).is_empty());

    let all = after.merge(looked);
    click(
        &mut window.dom,
        last(&all, "aria-label", "Send without encryption"),
    );
    let sent = queued(&store);
    assert_eq!(sent.len(), 1, "send without encryption did not send");
    let text = String::from_utf8_lossy(&sent[0]);
    assert!(!text.contains("multipart/encrypted"), "{text}");
    assert!(text.contains("see you there"), "{text}");
    assert_eq!(store.draft(draft.id).unwrap().openpgp, OpenPgp::None);
}

#[tokio::test]
async fn a_key_found_by_looking_up_clears_the_bar_and_the_send_goes_encrypted() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    crate::pgp::keys::generate(&store, secrets.as_ref(), "me@example.test", Utc::now()).unwrap();
    let dana = crate::ui::pgp::tests::someone_elses("dana@example.test", 91);
    let armored = dana.public().armored().unwrap();
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let mut seams = crate::ui::pgp::tests::seams_with(secrets);
    seams.lookup = {
        let asked = asked.clone();
        // What a domain's Web Key Directory answering would leave in the store.
        Arc::new(move |store, address| {
            asked.lock().unwrap().push(address.to_owned());
            let found = crate::pgp::keys::import(
                store,
                &MapSecrets::default(),
                armored.as_bytes(),
                Utc::now(),
            )
            .map_err(|e| e.to_string())?;
            Ok(found.into_iter().next().map(|one| one.key))
        })
    };
    let draft = asking(&store, OpenPgp::Encrypt);
    let (mut window, seen) = window_with(store.clone(), draft, seams);
    let after = click(&mut window.dom, seen.one("aria-label", "Send"));
    let mut looked = click(&mut window.dom, last(&after, "aria-label", "Look up keys"));
    let markup = until(&mut window, &mut looked, |page| page.contains("Found dana")).await;
    assert_eq!(*asked.lock().unwrap(), ["dana@example.test"]);
    assert!(!markup.contains("seal-warn"), "{markup}");
    assert!(queued(&store).is_empty(), "a lookup sent it");

    let mut sent = click(&mut window.dom, seen.one("aria-label", "Send"));
    let give_up = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while queued(&store).is_empty() && tokio::time::Instant::now() < give_up {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            window.dom.wait_for_work(),
        )
        .await;
        window.dom.render_immediate(&mut sent);
    }
    let out = queued(&store);
    assert_eq!(out.len(), 1);
    assert!(String::from_utf8_lossy(&out[0]).contains("multipart/encrypted"));
}

#[tokio::test]
async fn no_key_of_ones_own_offers_to_make_one() {
    let (store, _dir) = seeded();
    let draft = asking(&store, OpenPgp::Sign);
    let (mut window, seen) = window(store.clone(), draft, Arc::default());
    let after = click(&mut window.dom, seen.one("aria-label", "Send"));
    let markup = window.render();
    assert!(
        markup.contains("me@example.test has no OpenPGP key of its own"),
        "{markup}"
    );
    assert!(queued(&store).is_empty());
    click(&mut window.dom, after.one("aria-label", "Create a key…"));
    let shell = window.shell;
    assert!(window.dom.in_runtime(|| shell.peek().keys.is_some()));
}

#[tokio::test]
async fn a_locked_key_is_asked_for_in_the_bar_and_a_wrong_passphrase_said() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    let passphrase = "kestrel over the hill";
    let key = mail_mime::openpgp::generate(
        "Me <me@example.test>",
        Utc::now(),
        &mut rand::rngs::StdRng::seed_from_u64(81),
    )
    .unwrap()
    .with_passphrase(passphrase, &mut rand::rngs::StdRng::seed_from_u64(82))
    .unwrap();
    crate::pgp::keys::import(
        &store,
        secrets.as_ref(),
        key.armored().unwrap().as_bytes(),
        Utc::now(),
    )
    .unwrap();
    let draft = asking(&store, OpenPgp::Sign);
    let (mut window, seen) = window(store.clone(), draft, secrets);

    let mut after = click(&mut window.dom, seen.one("aria-label", "Send"));
    let prompt = format!(
        "Your OpenPGP key {} needs its passphrase to sign or encrypt this message.",
        crate::ui::pgp::short(key.fingerprint())
    );
    let markup = until(&mut window, &mut after, |page| page.contains(&prompt)).await;
    assert!(markup.contains(&prompt), "{markup}");
    assert!(queued(&store).is_empty());
    let page = window.page();
    assert!(matches!(
        window.dom.in_runtime(|| page.peek().seal_bar.clone()),
        SealBar::Locked { .. }
    ));

    let field = after.one("aria-label", &format!("Passphrase: {prompt}"));
    type_into(&mut window.dom, field, "not it");
    let mut wrong = click(&mut window.dom, after.one("aria-label", "Unlock and send"));
    let markup = until(&mut window, &mut wrong, |page| {
        page.contains("did not unlock")
    })
    .await;
    assert!(
        markup.contains("That passphrase did not unlock the key."),
        "{markup}"
    );
    assert!(queued(&store).is_empty());

    let all = after.merge(wrong);
    type_into(
        &mut window.dom,
        last(&all, "aria-label", &format!("Passphrase: {prompt}")),
        passphrase,
    );
    assert!(
        !window.render().contains(passphrase),
        "the passphrase is in the markup"
    );
    let shell = window.shell;
    assert!(
        !window
            .dom
            .in_runtime(|| format!("{:?} {:?}", shell.peek(), page.peek()))
            .contains(passphrase),
        "the passphrase is in the shell or the page"
    );
    let mut sent = click(&mut window.dom, last(&all, "aria-label", "Unlock and send"));
    let give_up = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while queued(&store).is_empty() && tokio::time::Instant::now() < give_up {
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            window.dom.wait_for_work(),
        )
        .await;
        window.dom.render_immediate(&mut sent);
    }
    let out = queued(&store);
    assert_eq!(out.len(), 1, "unlocked, it did not send");
    let text = String::from_utf8_lossy(&out[0]);
    assert!(text.contains("multipart/signed"), "{text}");
}

#[tokio::test]
async fn every_class_of_the_bar_and_row_is_styled() {
    let (store, _dir) = seeded();
    let draft = asking(&store, OpenPgp::Encrypt);
    let secrets = Arc::new(MapSecrets::default());
    crate::pgp::keys::generate(&store, secrets.as_ref(), "me@example.test", Utc::now()).unwrap();
    let (mut window, seen) = window(store, draft, secrets);
    click(&mut window.dom, seen.one("aria-label", "Send"));
    let mut markup = window.render();
    assert!(markup.contains("seal-warn"), "{markup}");
    let mut page = window.page();
    window.dom.in_runtime(|| {
        page.write().seal_bar = SealBar::Locked {
            key: Fingerprint::V4([1; 20]),
            tried: crate::ui::pgp::Tried::Wrong,
        };
    });
    markup += &window.render();
    assert!(markup.contains("unlock-field"), "{markup}");
    let missing = crate::ui::style::tests::unstyled_classes(&markup, crate::ui::style::STYLE);
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

#[tokio::test]
#[ignore = "writes target/shots/openpgp-composer.html for a person or a headless browser to look at"]
async fn render_the_composer_bar_to_a_file() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    crate::pgp::keys::generate(&store, secrets.as_ref(), "me@example.test", Utc::now()).unwrap();
    let mut body = String::new();
    let bars = [
        None,
        Some(SealBar::Locked {
            key: Fingerprint::V4([0x5C; 20]),
            tried: crate::ui::pgp::Tried::Wrong,
        }),
        Some(SealBar::Blind(
            crate::ui::pgp::Scheme::OpenPgp,
            vec!["cara@example.test".to_owned()],
        )),
    ];
    for bar in bars {
        let draft = asking(&store, OpenPgp::SignAndEncrypt);
        let (mut window, seen) = window(store.clone(), draft, secrets.clone());
        click(&mut window.dom, seen.one("aria-label", "Send"));
        if let Some(bar) = bar {
            let mut page = window.page();
            window.dom.in_runtime(|| page.write().seal_bar = bar);
        }
        body.push_str(&format!(
            "<section class=\"reader\" style=\"width:720px;height:520px;margin:16px\">{}</section>",
            window.render()
        ));
    }
    let shots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(shots).unwrap();
    crate::ui::fixtures::dump("shots/openpgp-composer", &body);
}
