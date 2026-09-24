//! What the reader keeps of an opened message: the attachments inside it, saved as they were
//! sent, and an opening asked again once the keys have changed — without the sheet's help.

use std::sync::Arc;

use mail_domain::*;
use mail_mime::openpgp::Cert;
use mail_runtime::MapSecrets;

use super::tests::{ME, arrive, own_key, reader, sealed, someone_elses, until};
use super::{Look, looks_at, lookup, save_attachment};
use crate::ui::fixtures::seeded;

/// A message to me saying `text`, with `map.bin` attached, unique by `word`.
fn letter(word: &str, text: &str) -> String {
    format!(
        "From: Bea <bea@example.test>\r\nTo: me@example.test\r\nSubject: {word} plans\r\n\
         Date: Thu, 24 Sep 2026 10:00:00 +0000\r\nMessage-ID: <{word}-{}@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n\
         --b\r\nContent-Type: application/octet-stream; name=\"map.bin\"\r\n\
         Content-Disposition: attachment; filename=\"map.bin\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nAAECAwQFBgc=\r\n--b--\r\n",
        uuid::Uuid::new_v4()
    )
}

#[tokio::test]
async fn an_openpgp_message_opened_lists_what_is_attached_inside_and_saves_it_whole() {
    let (store, dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    let key = own_key(&store, &secrets);
    let bea = someone_elses("bea@example.test", 111);
    let raw = letter("otter", "the otter map is attached");
    let to = Cert::from_bytes(&key.key).unwrap();
    let message = arrive(
        &store,
        sealed(&raw, OpenPgp::SignAndEncrypt, Some(&bea), &[to], 112),
    );
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| {
        page.contains("the otter map is attached") && page.contains("map.bin")
    })
    .await;
    // What was attached inside is listed; the ciphertext it arrived in is not.
    assert!(page.contains("map.bin"), "{page}");
    assert!(!page.contains("encrypted.asc"), "{page}");
    let saved =
        save_attachment(message.id, message.body.raw(), 0, &dir.path().join("out")).unwrap();
    assert_eq!(saved.file_name().unwrap(), "map.bin");
    assert_eq!(std::fs::read(saved).unwrap(), [0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn a_key_arriving_by_any_road_opens_a_message_again_without_the_sheet() {
    let (store, _dir) = seeded();
    let secrets = MapSecrets::default();
    // A key of mine, made elsewhere and not yet here.
    let key = someone_elses(ME, 121);
    let raw = letter("marten", "the marten waits");
    let message = arrive(
        &store,
        sealed(&raw, OpenPgp::Encrypt, None, &[key.public()], 122),
    );
    let body = message.body.raw();
    let Look::Opened(first) = lookup(&store, &secrets, message.id, body) else {
        panic!("not opened");
    };
    assert!(first.shown.is_none(), "read without the key");
    let before = looks_at(message.id);

    // Imported straight through the data side, as `mailo pgp import` would: nothing tells the
    // window, and nothing has to.
    crate::pgp::keys::import(
        &store,
        &secrets,
        key.armored().unwrap().as_bytes(),
        chrono::Utc::now(),
    )
    .unwrap();
    let Look::Opened(second) = lookup(&store, &secrets, message.id, body) else {
        panic!("not opened");
    };
    assert!(looks_at(message.id) > before, "the old look was kept");
    let text = second
        .shown
        .and_then(|shown| shown.text)
        .unwrap_or_default();
    assert!(text.contains("the marten waits"), "{text}");
}
