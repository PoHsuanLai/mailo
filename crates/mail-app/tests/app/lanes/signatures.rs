//! Signatures: one made in Settings › Signatures, written in the composer's own body there, signs
//! the next new message in the main window, open all along, which offers it in its Signature row.

use mail_app::ui::copy;
use mail_core::Store;

use super::drive::Key;
use super::hands::{click, type_text, until};
use super::seed::ME;
use super::window::{Window, account};

/// The editor on a signature's page.
const EDITOR: &str = ".sig-edit .c-body";

#[test]
fn a_signature_made_in_settings_signs_a_new_message() {
    let mut window = Window::open(|_| {});
    let store = window.store.clone();
    let signatures = || mail_core::signature::every(&store).unwrap();
    assert!(
        signatures().iter().all(|(_, kept)| kept.is_empty()),
        "signed before anything was made"
    );

    window.in_settings(copy::titles::SIGNATURES, |settings| {
        let new = format!("[*|aria-label=\"{}\"]", copy::aria::new_signature_for(ME));
        until(settings, "the address offers New Signature", |h| {
            h.count(&new) == 1
        });
        click(settings, &new);
        until(settings, "the new signature's page is pushed", |h| {
            h.count(EDITOR) == 1
        });
        click(settings, EDITOR);
        type_text(settings, "Ada Lovelace");
        until(settings, "the words are kept as they are typed", |_| {
            signatures()
                .iter()
                .flat_map(|(_, kept)| kept.iter())
                .any(|kept| kept.text == "Ada Lovelace")
        });
    });

    let made = signatures()
        .into_iter()
        .flat_map(|(_, kept)| kept)
        .collect::<Vec<_>>();
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(
        (made[0].new_mail, made[0].replies),
        (
            mail_domain::IsDefault::Default,
            mail_domain::IsDefault::Default
        ),
        "an address's first signature is its default"
    );
    assert!(
        made[0].html.is_some(),
        "written in the editor, kept as its HTML too"
    );

    // The main window, open all along: a new message ends with it, under the separator, and
    // its Signature row names it.
    let drafts = || store.drafts(account()).unwrap().len();
    let before = drafts();
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer signed with it", |h| {
        h.count(".cpage .c-body") == 1
            && h.text_of(".cpage .c-body")
                .is_some_and(|body| body.contains("Ada Lovelace"))
    });
    window.until("the Signature row names it", |h| {
        h.text_of(".c-props [*|data-row=signature]")
            .is_some_and(|row| row.contains(mail_domain::UNNAMED_SIGNATURE))
    });
    assert_eq!(drafts(), before + 1);
    let draft = store
        .drafts(account())
        .unwrap()
        .into_iter()
        .max_by_key(|draft| draft.updated)
        .expect("the new message's draft");
    assert!(
        draft.text.ends_with("-- \r\nAda Lovelace\r\n"),
        "{:?}",
        draft.text
    );
}
