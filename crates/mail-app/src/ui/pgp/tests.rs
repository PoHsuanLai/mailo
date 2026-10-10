//! OpenPGP in the reader: every verdict said in words over the body it belongs to, the passphrase
//! asked for inline, and the passphrase itself nowhere it could be read back.
//!
//! Against a real store, with the keyring a [`MapSigningStore`] handed in as the window's [`Seams`],
//! so the user's own is never touched.

use std::cell::Cell;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_core::message::{Looks, Read};
use mail_core::{Arrival, MapSigningStore};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use mail_mime::openpgp::{self, Cert, SecretCert, Unlocking};
use rand::SeedableRng;

use super::{Said, Seams, Tone, said};
use crate::ui::fixtures::{
    Seen, acct_account, click, dispatching, rebuild_into, seeded, type_into,
};
use crate::ui::reading::Reader;
use crate::ui::view::Shell;
use mail_core::pgp::Protected;

pub(super) const ME: &str = "me@example.test";
const BEA: &str = "bea@example.test";

fn now() -> DateTime<Utc> {
    Utc::now()
}

/// The window's seams, with `secrets` as the keyring and nothing else reachable.
pub(in crate::ui) fn seams_with(secrets: Arc<MapSigningStore>) -> Seams {
    Seams {
        secrets,
        lookup: Arc::new(|_, _| Err("no lookups in tests".to_owned())),
        pick: Arc::new(|| None),
        save: Arc::new(|_| None),
    }
}

/// A key of the user's own, made the way the sheet makes one.
pub(super) fn own_key(store: &SqliteStore, secrets: &MapSigningStore) -> PgpKey {
    mail_core::pgp::keys::generate(store, secrets, ME, now()).unwrap()
}

/// A correspondent's key, made outside this client.
pub(in crate::ui) fn someone_elses(address: &str, seed: u64) -> SecretCert {
    openpgp::generate(
        &format!("Them <{address}>"),
        now(),
        &mut rand::rngs::StdRng::seed_from_u64(seed),
    )
    .unwrap()
}

/// `raw` sealed as `mode`, signed by `signer` when given, to `to`.
pub(super) fn sealed(
    raw: &str,
    mode: OpenPgp,
    signer: Option<&SecretCert>,
    to: &[Cert],
    seed: u64,
) -> Vec<u8> {
    let signer = signer.map(|key| Unlocking {
        key: key.clone(),
        passphrase: String::new(),
    });
    openpgp::seal(
        raw.as_bytes(),
        &openpgp::Sealing {
            mode,
            signer: signer.as_ref(),
            recipients: to,
            gossip: &[],
            now: now(),
            boundary_prefix: "mailo",
        },
        &mut rand::rngs::StdRng::seed_from_u64(seed),
    )
    .unwrap()
}

/// A plain message from bea saying `text`, unique by `word`.
fn letter(word: &str, text: &str) -> String {
    format!(
        "From: Bea <bea@example.test>\r\nTo: me@example.test\r\nSubject: {word} plan\r\n\
         Message-ID: <{word}-{}@example.test>\r\nDate: Thu, 24 Sep 2026 16:00:00 +0000\r\n\
         MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n",
        uuid::Uuid::new_v4()
    )
}

/// Store `raw` as a message that arrived in the inbox, the way a sync does.
pub(super) fn arrive(store: &SqliteStore, raw: Vec<u8>) -> Message {
    let ingest = mail_core::assemble(
        store,
        acct_account(),
        MailboxRef {
            account: acct_account(),
            path: "INBOX".to_owned(),
        },
        MailboxRole::Inbox,
        None,
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uuid::Uuid::new_v4().to_string(),
            },
            raw,
        }],
        now(),
    )
    .unwrap();
    let id = ingest.messages[0].message.id;
    store.ingest(acct_account(), ingest).unwrap();
    store.message(id).unwrap()
}

/// My own secret key, as the keyring holds it.
pub(super) fn mine(secrets: &MapSigningStore, key: &PgpKey) -> SecretCert {
    mail_runtime::pgp::secret_key(secrets, acct_account(), key.fingerprint).unwrap()
}

thread_local! {
    static SHELL: Cell<Option<Signal<Shell>>> = const { Cell::new(None) };
}

#[component]
fn Open(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    SHELL.with(|slot| slot.set(Some(shell)));
    rsx! { ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window, Reader { thread, shell } } }
}

/// The reader on `thread`, with `secrets` as the keyring, and the lookups it uses: the ones the
/// seal and the reader's own body share, as the launched window's do.
pub(super) fn reader_looking(
    store: Arc<SqliteStore>,
    secrets: Arc<MapSigningStore>,
    thread: ThreadId,
) -> (VirtualDom, Seen, Arc<Looks>) {
    dispatching();
    let looks = Arc::new(Looks::new());
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread })
        .with_root_context(store)
        .with_root_context(looks.clone())
        .with_root_context(seams_with(secrets));
    let seen = rebuild_into(&mut dom);
    (dom, seen, looks)
}

/// [`reader_looking`], without the lookups.
pub(super) fn reader(
    store: Arc<SqliteStore>,
    secrets: Arc<MapSigningStore>,
    thread: ThreadId,
) -> (VirtualDom, Seen) {
    let (dom, seen, _) = reader_looking(store, secrets, thread);
    (dom, seen)
}

pub(super) fn markup(dom: &VirtualDom) -> String {
    dioxus_ssr::render(dom).replace("&#39;", "'")
}

/// Let the dom's tasks run until `done` holds of its markup, keeping every attribute the renders
/// set, or give up after fifteen seconds: a protected key's passphrase is slow to check on
/// purpose.
pub(super) async fn until(
    dom: &mut VirtualDom,
    seen: &mut Seen,
    done: impl Fn(&str) -> bool,
) -> String {
    let give_up = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let page = markup(dom);
        if done(&page) || tokio::time::Instant::now() > give_up {
            return page;
        }
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(100), dom.wait_for_work()).await;
        dom.render_immediate(seen);
    }
}

/// The seal's lines, in order, as `(class, words)`.
/// `page` without the list rows that are leaving: a quire `List` keeps a removed row drawn while
/// it fades out, so what a list holds now is what is not `data-presence="leaving"`.
pub(super) fn without_leaving(page: &str) -> String {
    let mut out = String::new();
    let mut rest = page;
    while let Some(at) = rest.find("data-presence=\"leaving\"") {
        let start = rest[..at].rfind("<div").unwrap_or(at);
        out.push_str(&rest[..start]);
        let mut depth = 0usize;
        let mut end = start;
        for (offset, piece) in rest[start..].match_indices(['<']) {
            let tail = &rest[start + offset..];
            if tail.starts_with("<div") {
                depth += 1;
            } else if tail.starts_with("</div") {
                depth -= 1;
                if depth == 0 {
                    end = start + offset + "</div>".len();
                    break;
                }
            }
            let _ = piece;
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// What the reader says about a message's protection, in the order it says it: each line's
/// look (`seal-line`, then `good`, `unknown`, `warn`, `bad` or `detail` after a space) and its
/// words. The lines are quire's: a `Label` in a status colour (good, unknown), a plain or quiet
/// one, or an `InlineBanner` (warn, bad), directly under the `.seal` box; the passphrase field
/// under it is not a line.
pub(super) fn lines(page: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = page;
    while let Some(at) = rest.find("class=\"seal\"") {
        let open = rest[..at].rfind("<div").unwrap_or(at);
        let end = close_of(&rest[open..]);
        out.extend(seal_lines(&rest[open..open + end]));
        rest = &rest[open + end..];
    }
    out
}

/// Where the element that opens `html` ends: after its matching `</div>`.
fn close_of(html: &str) -> usize {
    let (mut depth, mut at) = (0_usize, 0_usize);
    while let Some(next) = html[at..].find(['<']) {
        at += next;
        if html[at..].starts_with("</div") {
            depth -= 1;
            at += html[at..].find('>').map_or(html.len() - at, |end| end + 1);
            if depth == 0 {
                return at;
            }
        } else {
            if html[at..].starts_with("<div") {
                depth += 1;
            }
            at += 1;
        }
    }
    html.len()
}

/// The text inside `html`, without its tags.
fn words_of(html: &str) -> String {
    let (mut words, mut in_tag) = (String::new(), false);
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => words.push(c),
            _ => {}
        }
    }
    words
}

fn seal_lines(seal: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Past the box's own opening tag.
    let mut rest = &seal[seal.find('>').map_or(0, |end| end + 1)..];
    while let Some(at) = rest.find('<') {
        rest = &rest[at..];
        let tag_end = rest.find('>').unwrap_or(rest.len() - 1);
        let tag = &rest[..=tag_end];
        if rest.starts_with("<span") && tag.contains("class=\"ds-label\"") {
            let end = rest.find("</span>").unwrap_or(rest.len());
            let look = match (severity_of(tag), tag.contains("data-role=\"tertiary\"")) {
                (Some("ok"), _) => "seal-line good",
                (Some("warn"), _) => "seal-line unknown",
                (None, true) => "seal-line detail",
                _ => "seal-line",
            };
            out.push((look.to_owned(), words_of(&rest[tag_end + 1..end])));
            rest = &rest[end..];
        } else if rest.starts_with("<div") {
            let end = close_of(rest);
            if tag.contains("ds-inline-banner\"") {
                let body = rest.find("ds-inline-banner-body").unwrap_or(0);
                let inner = &rest[body..end];
                let words = inner.find("</span>").map_or(inner, |close| &inner[..close]);
                let look = match severity_of(tag) {
                    Some("danger") => "seal-line bad",
                    Some("warn") => "seal-line warn",
                    _ => "seal-line",
                };
                out.push((
                    look.to_owned(),
                    words_of(&words[words.find('>').map_or(0, |at| at + 1)..]),
                ));
            }
            rest = &rest[end..];
        } else {
            rest = &rest[1..];
        }
    }
    out
}

/// The `data-severity` a tag carries.
fn severity_of(tag: &str) -> Option<&str> {
    let at = tag.find("data-severity=\"")? + "data-severity=\"".len();
    tag[at..].split('"').next()
}

/// Whether the reader draws a line that looks like `class` (`seal-line` alone: any line).
pub(super) fn shows(page: &str, class: &str) -> bool {
    lines(page)
        .iter()
        .any(|(look, _)| class == "seal-line" || look == class)
}

#[test]
fn every_verdict_is_said_in_words_and_a_bad_one_is_unmissable() {
    let signer = PgpKey {
        fingerprint: Fingerprint::V4([0xAB; 20]),
        key_ids: vec![],
        user_ids: vec!["Bea <bea@example.test>".to_owned()],
        emails: vec![BEA.to_owned()],
        key: vec![],
        source: KeySource::Imported,
        first_seen: now(),
        last_seen: now(),
        trust: KeyTrust::Unverified,
        created: None,
        expires: None,
        secret: SecretHeld::Absent,
    };
    let protected = |encryption, verification| Protected {
        encryption,
        verification,
        signer: Some(signer.clone()),
        shown: None,
        raw: None,
    };
    let good = |trust, coverage| Verification::Good {
        signer: signer.fingerprint,
        trust,
        coverage,
    };
    let line = |tone, text: &str| Said {
        tone,
        text: text.to_owned(),
    };
    let cases = [
        (
            "signed and good",
            protected(
                Encryption::NotEncrypted,
                good(KeyTrust::Unverified, Coverage::Whole),
            ),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Good,
                    "Good signature by Bea <bea@example.test> · ABAB ABAB ABAB ABAB · not verified by you",
                ),
            ],
        ),
        (
            "verified",
            protected(
                Encryption::Decrypted,
                good(KeyTrust::Verified, Coverage::Whole),
            ),
            vec![
                line(Tone::Good, "Encrypted, and decrypted for reading"),
                line(
                    Tone::Good,
                    "Good signature by Bea <bea@example.test> · ABAB ABAB ABAB ABAB · verified by you",
                ),
            ],
        ),
        (
            "bad",
            protected(
                Encryption::NotEncrypted,
                Verification::Bad {
                    coverage: Coverage::Whole,
                },
            ),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Bad,
                    "Bad signature: the message was changed after it was signed, or the \
                     signature is forged",
                ),
            ],
        ),
        (
            "unknown key, part signed",
            protected(
                Encryption::NotEncrypted,
                Verification::UnknownKey {
                    issuer: KeyId([0x12; 8]),
                    coverage: Coverage::Part,
                },
            ),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Unknown,
                    "Signed by key 1212 1212 1212 1212, which you don’t have",
                ),
                line(Tone::Unknown, "Only part of this message is signed"),
            ],
        ),
        (
            "encrypted to someone else",
            protected(
                Encryption::CannotDecrypt {
                    to: vec![KeyId([0x34; 8])],
                },
                Verification::NoSignature,
            ),
            vec![
                line(
                    Tone::Unknown,
                    "Encrypted to keys you don’t have (3434 3434 3434 3434)",
                ),
                line(Tone::Plain, "Not signed"),
            ],
        ),
        (
            "unreadable",
            protected(
                Encryption::Unreadable {
                    why: "tampered".to_owned(),
                },
                Verification::NoSignature,
            ),
            vec![
                line(Tone::Bad, "Encrypted, and could not be read: tampered"),
                line(Tone::Plain, "Not signed"),
            ],
        ),
    ];
    for (case, given, want) in cases {
        assert_eq!(said(&given), want, "{case}");
    }
}

/// Each tone has its own look, and no two share one: a signature that is believed, one nobody
/// can check, one believed only in part, and one that does not check are four different things.
#[test]
fn every_tone_draws_its_own_look() {
    let tones = [
        Tone::Plain,
        Tone::Good,
        Tone::Unknown,
        Tone::Warn,
        Tone::Bad,
        Tone::Detail,
    ];
    let mut dom = VirtualDom::new_with_props(
        |tones: [Tone; 6]| {
            rsx! {
                div { class: "seal",
                    for (at, tone) in tones.into_iter().enumerate() {
                        {super::seal::said_line(at, super::said::Said { tone, text: format!("line {at}") })}
                    }
                }
            }
        },
        tones,
    );
    dom.rebuild_in_place();
    let page = dioxus_ssr::render(&dom);
    let looks: Vec<String> = lines(&page).into_iter().map(|(look, _)| look).collect();
    assert_eq!(
        looks,
        [
            "seal-line",
            "seal-line good",
            "seal-line unknown",
            "seal-line warn",
            "seal-line bad",
            "seal-line detail"
        ]
    );
}

#[tokio::test]
async fn a_signed_message_says_who_signed_it_over_its_body() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let key = own_key(&store, &secrets);
    let raw = letter("owl", "the owl note is signed");
    let message = arrive(
        &store,
        sealed(&raw, OpenPgp::Sign, Some(&mine(&secrets, &key)), &[], 21),
    );
    let (mut dom, mut seen, looks) = reader_looking(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| shows(page, "seal-line")).await;
    assert!(looks.reads(Read::Seal) > 0);
    let said = lines(&page);
    assert_eq!(
        said[0],
        ("seal-line".to_owned(), "Not encrypted".to_owned())
    );
    assert_eq!(said[1].0, "seal-line good");
    assert!(
        said[1]
            .1
            .starts_with("Good signature by &#60;me@example.test&#62; · ")
            && said[1].1.ends_with("not verified by you"),
        "{said:?}"
    );
    assert!(page.contains("the owl note is signed"), "{page}");
    assert!(!page.contains("BEGIN PGP SIGNATURE"), "{page}");
}

#[tokio::test]
async fn a_bad_signature_is_said_on_the_danger_ground() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let key = own_key(&store, &secrets);
    let raw = letter("heron", "pay the heron invoice");
    let signed = sealed(&raw, OpenPgp::Sign, Some(&mine(&secrets, &key)), &[], 22);
    // Changed after it was signed.
    let text = String::from_utf8(signed).unwrap();
    assert!(text.contains("pay the heron invoice"));
    let tampered = text.replace("pay the heron invoice", "pay the forged invoice");
    let message = arrive(&store, tampered.into_bytes());
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| shows(page, "seal-line")).await;
    let said = lines(&page);
    assert_eq!(said[1].0, "seal-line bad", "{said:?}");
    assert!(said[1].1.starts_with("Bad signature"), "{said:?}");
    assert!(page.contains("pay the forged invoice"), "{page}");
}

#[tokio::test]
async fn a_signature_by_a_key_not_held_cannot_be_checked_and_does_not_look_good() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let bea = someone_elses(BEA, 23);
    let raw = letter("lynx", "the lynx is signed by bea");
    let message = arrive(&store, sealed(&raw, OpenPgp::Sign, Some(&bea), &[], 24));
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| shows(page, "seal-line")).await;
    let said = lines(&page);
    let issuer = super::grouped(&bea.fingerprint().key_id().to_string());
    assert_eq!(
        said[1],
        (
            "seal-line unknown".to_owned(),
            format!("Signed by key {issuer}, which you don’t have")
        )
    );
    assert!(!shows(&page, "seal-line good"), "{page}");
    assert!(page.contains("the lynx is signed by bea"), "{page}");
}

#[tokio::test]
async fn an_encrypted_message_shows_what_it_was_encrypted_to_say() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let key = own_key(&store, &secrets);
    let bea = someone_elses(BEA, 25);
    let raw = letter("zebra", "the zebra is under the mat");
    let to = Cert::from_bytes(&key.key).unwrap();
    let message = arrive(
        &store,
        sealed(&raw, OpenPgp::SignAndEncrypt, Some(&bea), &[to], 26),
    );
    assert!(!message.body.text().unwrap_or_default().contains("zebra"));
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| {
        page.contains("the zebra is under the mat")
    })
    .await;
    let said = lines(&page);
    assert_eq!(
        said[0],
        (
            "seal-line good".to_owned(),
            "Encrypted, and decrypted for reading".to_owned()
        )
    );
    assert_eq!(
        said[1].0, "seal-line unknown",
        "bea's key is not held: {said:?}"
    );
    assert!(page.contains("the zebra is under the mat"), "{page}");
    assert!(
        page.contains("zebra plan"),
        "the subject from inside: {page}"
    );
    assert!(
        !page.contains("encrypted.asc"),
        "the ciphertext offered as an attachment: {page}"
    );
}

/// A message to a key of mine that has a passphrase, and that key.
fn locked(
    store: &SqliteStore,
    secrets: &MapSigningStore,
    word: &str,
    seed: u64,
) -> (Message, String, Fingerprint) {
    let passphrase = format!("{word} battery staple");
    let key = someone_elses(ME, seed)
        .with_passphrase(
            &passphrase,
            &mut rand::rngs::StdRng::seed_from_u64(seed + 1),
        )
        .unwrap();
    mail_core::pgp::keys::import(store, secrets, key.armored().unwrap().as_bytes(), now()).unwrap();
    let raw = letter(word, &format!("the {word} sleeps at noon"));
    let message = arrive(
        store,
        sealed(&raw, OpenPgp::Encrypt, None, &[key.public()], seed + 2),
    );
    (message, passphrase, key.fingerprint())
}

const ASKED: &str = "Passphrase for key";

/// The passphrase field for `key`, found by its label.
fn field(seen: &Seen, key: Fingerprint) -> dioxus_core::ElementId {
    seen.one(
        "aria-label",
        &format!("Passphrase: Passphrase for key {}", super::short(key)),
    )
}

#[tokio::test]
async fn a_locked_key_is_asked_for_inline_and_opens_with_its_passphrase() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let (message, passphrase, key) = locked(&store, &secrets, "otter", 31);
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| page.contains(ASKED)).await;
    assert!(page.contains(ASKED), "{page}");
    assert!(!page.contains("the otter sleeps at noon"), "{page}");
    assert!(!page.contains("Wrong passphrase"), "{page}");

    let mut typed = type_into(&mut dom, field(&seen, key), &passphrase);
    let page = markup(&dom);
    assert!(
        !page.contains(&passphrase),
        "the passphrase is in the markup"
    );
    let shell = SHELL.with(Cell::get).unwrap();
    let debug = dom.in_runtime(|| format!("{:?}", shell.peek()));
    assert!(
        !debug.contains(&passphrase),
        "the passphrase is in the shell"
    );

    let pressed = click(&mut dom, seen.one("aria-label", "Unlock"));
    typed = typed.merge(pressed);
    let page = until(&mut dom, &mut typed, |page| {
        page.contains("the otter sleeps at noon")
    })
    .await;
    assert!(page.contains("the otter sleeps at noon"), "{page}");
    assert_eq!(
        lines(&page)[0],
        (
            "seal-line good".to_owned(),
            "Encrypted, and decrypted for reading".to_owned()
        )
    );
    assert!(
        !page.contains(&passphrase),
        "the passphrase is in the markup"
    );
    let debug = dom.in_runtime(|| format!("{:?}", shell.peek()));
    assert!(
        !debug.contains(&passphrase),
        "the passphrase is in the shell"
    );
}

#[tokio::test]
async fn a_wrong_passphrase_says_so_and_can_be_tried_again() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let (message, passphrase, key) = locked(&store, &secrets, "badger", 41);
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    until(&mut dom, &mut seen, |page| page.contains(ASKED)).await;

    type_into(&mut dom, field(&seen, key), "not the words");
    let mut after = click(&mut dom, seen.one("aria-label", "Unlock"));
    let page = until(&mut dom, &mut after, |page| {
        page.contains("Wrong passphrase")
    })
    .await;
    assert!(page.contains("Wrong passphrase."), "{page}");
    assert!(!page.contains("the badger sleeps at noon"), "{page}");

    // The field and its button are drawn again, and the right words open it.
    let seen = seen.merge(after);
    type_into(&mut dom, field(&seen, key), &passphrase);
    let mut last = click(&mut dom, seen.one("aria-label", "Unlock"));
    let page = until(&mut dom, &mut last, |page| {
        page.contains("the badger sleeps at noon")
    })
    .await;
    assert!(page.contains("the badger sleeps at noon"), "{page}");
    assert!(!page.contains("Wrong passphrase"), "{page}");
}

#[tokio::test]
async fn a_signature_on_only_part_of_a_message_says_so() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let key = own_key(&store, &secrets);
    let signed = sealed(
        &letter("crane", "the crane part is signed"),
        OpenPgp::Sign,
        Some(&mine(&secrets, &key)),
        &[],
        51,
    );
    // The signed entity as a mailing list forwards it: under a multipart/mixed, beside a footer
    // nobody signed.
    let signed = String::from_utf8(signed).unwrap();
    let (head, body) = signed.split_once("\r\n\r\n").unwrap();
    let content_type = head
        .split("\r\n")
        .skip_while(|line| !line.to_ascii_lowercase().starts_with("content-type:"))
        .take_while(|line| {
            line.to_ascii_lowercase().starts_with("content-type:") || line.starts_with([' ', '\t'])
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    let raw = format!(
        "From: Bea <bea@example.test>\r\nTo: me@example.test\r\nSubject: crane list\r\n\
         Message-ID: <crane-{}@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n\
         --outer\r\n{content_type}\r\n\r\n{body}\r\n--outer\r\n\
         Content-Type: text/plain\r\n\r\nunsubscribe from the crane list\r\n--outer--\r\n",
        uuid::Uuid::new_v4()
    );
    let message = arrive(&store, raw.into_bytes());
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| shows(page, "seal-line")).await;
    let said = lines(&page);
    assert_eq!(said[1].0, "seal-line good", "{said:?}");
    assert_eq!(
        said.last().unwrap(),
        &(
            "seal-line unknown".to_owned(),
            "Only part of this message is signed".to_owned()
        )
    );
    assert!(page.contains("the crane part is signed"), "{page}");
}

#[tokio::test]
async fn a_plain_message_says_nothing_and_every_seal_class_is_styled() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let plain = arrive(&store, letter("finch", "no openpgp here").into_bytes());
    let (mut dom, mut seen, looks) = reader_looking(store.clone(), secrets.clone(), plain.thread);
    let page = until(&mut dom, &mut seen, |_| looks.reads(Read::Seal) > 0).await;
    assert!(!page.contains("class=\"seal"), "{page}");

    let key = own_key(&store, &secrets);
    let (locked_one, _, _) = locked(&store, &secrets, "stoat", 61);
    let (mut dom, mut seen) = reader(store.clone(), secrets.clone(), locked_one.thread);
    let mut page = until(&mut dom, &mut seen, |page| page.contains(ASKED)).await;
    let raw = letter("wren", "wren");
    let bad = String::from_utf8(sealed(
        &raw,
        OpenPgp::Sign,
        Some(&mine(&secrets, &key)),
        &[],
        62,
    ))
    .unwrap()
    .replace("\r\nwren\r\n", "\r\nwrong\r\n");
    let bad = arrive(&store, bad.into_bytes());
    let (mut dom, mut seen) = reader(store, secrets, bad.thread);
    page += &until(&mut dom, &mut seen, |page| shows(page, "seal-line bad")).await;
    assert!(shows(&page, "seal-line bad"), "{page}");
    let drawn = seals(&page);
    assert!(
        drawn.contains("unlock-field") && shows(&drawn, "seal-line bad"),
        "{drawn}"
    );
    let offences = crate::ui::style::tests::markup_offences(&drawn);
    assert!(offences.is_empty(), "the markup lint: {offences:#?}");
}

/// Every `div.seal` in `page`, whole: what this module draws, and nothing of the reader around it.
pub(super) fn seals(page: &str) -> String {
    let mut out = String::new();
    let mut rest = page;
    while let Some(at) = rest.find("<div class=\"seal\"") {
        rest = &rest[at..];
        let mut depth = 0usize;
        let mut end = 0;
        while end < rest.len() {
            if rest.as_bytes()[end..].starts_with(b"<div") {
                depth += 1;
            } else if rest.as_bytes()[end..].starts_with(b"</div>") {
                depth -= 1;
                if depth == 0 {
                    end += "</div>".len();
                    break;
                }
            }
            end += 1;
        }
        out.push_str(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

#[tokio::test]
#[ignore = "writes target/shots/openpgp.html for a person or a headless browser to look at"]
async fn render_the_reader_badges_and_the_keys_sheet_to_a_file() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSigningStore::default());
    let key = own_key(&store, &secrets);
    let bea = someone_elses(BEA, 101);
    let good = arrive(
        &store,
        sealed(
            &letter("good", "signed by me, and it checks"),
            OpenPgp::Sign,
            Some(&mine(&secrets, &key)),
            &[],
            102,
        ),
    );
    let unknown = arrive(
        &store,
        sealed(
            &letter("unknown", "encrypted to me, signed by a key I do not hold"),
            OpenPgp::SignAndEncrypt,
            Some(&bea),
            &[Cert::from_bytes(&key.key).unwrap()],
            103,
        ),
    );
    let bad = String::from_utf8(sealed(
        &letter("bad", "pay the usual account"),
        OpenPgp::Sign,
        Some(&mine(&secrets, &key)),
        &[],
        104,
    ))
    .unwrap()
    .replace("pay the usual account", "pay the other account");
    let bad = arrive(&store, bad.into_bytes());
    let (locked_one, _, locked_key) = locked(&store, &secrets, "locked", 105);
    let mut body = String::new();
    for (message, done) in [
        (good, "seal-line good"),
        (unknown, "seal-line unknown"),
        (bad, "seal-line bad"),
        (locked_one, ASKED),
    ] {
        let (mut dom, mut seen) = reader(store.clone(), secrets.clone(), message.thread);
        until(&mut dom, &mut seen, |page| page.contains(done)).await;
        if done == ASKED {
            type_into(&mut dom, field(&seen, locked_key), "wrong");
            let mut after = click(&mut dom, seen.one("aria-label", "Unlock"));
            until(&mut dom, &mut after, |page| {
                page.contains("Wrong passphrase")
            })
            .await;
        }
        body.push_str(&format!(
            "<section class=\"reader\" style=\"width:640px;height:340px;margin:16px\">{}</section>",
            markup(&dom)
        ));
    }
    let (mut sheet, seen) = super::keys_tests::sheet(&store, seams_with(secrets));
    super::keys_tests::asking(
        &mut sheet,
        seen.one(
            "aria-label",
            &format!("Delete {}", super::short(key.fingerprint)),
        ),
    );
    body.push_str(&format!(
        "<div class=\"app\" style=\"position:relative;width:760px;height:560px;margin:16px\">{}</div>",
        dioxus_ssr::render(&sheet)
    ));
    let shots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(shots).unwrap();
    crate::ui::fixtures::dump("shots/openpgp", &body);
}
