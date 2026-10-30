use super::*;
use crate::compose::{draft_new, draft_reply_placed, move_draft_to};
use crate::support_tests::{account, at, sqlite_with};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{MessageId, ReplyScope};
use mail_editor::{Node, ParaKind};

const ME: &str = "me@example.test";
const ELSEWHERE: &str = "me@elsewhere.test";

/// The message `sqlite_with` writes first.
fn first_message() -> MessageId {
    MessageId::from_uuid(uuid::Uuid::from_u128(0x9000))
}

fn other_account() -> AccountId {
    account_id_from_uuid(uuid::Uuid::from_u128(0xA2))
}

/// A signature of `identity`'s called `name`, its words `text`, written in the composer: bold.
fn rich(
    identity: IdentityId,
    name: &str,
    text: &str,
    new_mail: IsDefault,
    replies: IsDefault,
) -> Signature {
    Signature {
        id: SignatureId::generate(),
        identity,
        name: name.to_owned(),
        html: Some(format!("<p><b>{text}</b></p>")),
        text: text.to_owned(),
        new_mail,
        replies,
    }
}

/// A store with one message and two accounts, each sending as one identity with signatures:
/// the first's for everything, and the second's one for new mail and one for replies.
fn two_accounts() -> (SqliteStore, IdentityId, IdentityId) {
    let store = sqlite_with(&[("Lunch", "Shall we?", 0)]);
    mail_store::testing::seed_account(&store, other_account(), ELSEWHERE);
    let here = mail_store::testing::seed_default_identity(&store, account(), ME, Some("Ada"));
    let there =
        mail_store::testing::seed_default_identity(&store, other_account(), ELSEWHERE, None);
    use IsDefault::{Alternate, Default};
    for signature in [
        rich(here, "Work", "Ada at Work", Default, Default),
        rich(there, "Home", "Ada at Home", Default, Alternate),
        rich(there, "Brief", "A.", Alternate, Default),
    ] {
        store.put_signature(&signature).unwrap();
    }
    (store, here, there)
}

/// A new message starts with the identity's default for new mail, under the separator, in the
/// text part and with its formatting in the HTML part; moved to another account (From), it is
/// that identity's default instead, and what was written above stays.
#[test]
fn a_new_message_is_signed_with_the_default_and_signed_again_when_from_changes() {
    let (store, here, there) = two_accounts();
    let draft = draft_new(&store, account(), &[], "Plans", "See you at noon", at(1)).unwrap();
    assert_eq!(draft.identity, here);
    assert!(
        draft
            .text
            .ends_with("See you at noon\r\n\r\n-- \r\nAda at Work\r\n"),
        "{:?}",
        draft.text
    );
    let html = draft
        .html
        .as_deref()
        .expect("a signature with HTML gives the draft some");
    assert!(
        html.contains("<p>-- </p><p><strong>Ada at Work</strong></p>"),
        "{html}"
    );
    assert_eq!(draft.written(), "See you at noon");

    let moved = move_draft_to(&store, draft.id, other_account(), at(2)).unwrap();
    assert_eq!(
        (moved.account.clone(), moved.identity),
        (other_account(), there)
    );
    assert!(moved.text.contains("Ada at Home"), "{:?}", moved.text);
    assert!(!moved.text.contains("Ada at Work"), "{:?}", moved.text);
    assert_eq!(moved.written(), "See you at noon");
    assert_eq!(
        store.draft(draft.id).unwrap(),
        moved,
        "what was moved is what is stored"
    );

    // And back: the first identity's own again.
    let back = move_draft_to(&store, draft.id, account(), at(3)).unwrap();
    assert!(back.text.contains("Ada at Work"), "{:?}", back.text);
    assert!(!back.text.contains("Ada at Home"), "{:?}", back.text);
}

/// A signature the person changed in the draft is not one of the identity's, so all of it is
/// replaced; an identity with no default takes the signature off.
#[test]
fn an_edited_signature_is_replaced_whole_and_no_default_leaves_none() {
    let (store, _, there) = two_accounts();
    for signature in store.signatures(there).unwrap() {
        store.delete_signature(signature.id).unwrap();
    }
    let mut draft = draft_new(&store, account(), &[], "", "Hi", at(1)).unwrap();
    draft.text = draft.text.replace("Ada at Work", "Ada, at work today");
    draft.html = None;
    crate::compose::save(&store, &draft).unwrap();

    let moved = move_draft_to(&store, draft.id, other_account(), at(2)).unwrap();
    assert!(!moved.text.contains("-- "), "{:?}", moved.text);
    assert!(!moved.text.contains("at work"), "{:?}", moved.text);
    assert_eq!(moved.written(), "Hi");
}

/// A reply is signed with the default for replies, under the quoted original unless the
/// signature is asked above it.
#[test]
fn a_reply_takes_the_default_for_replies_where_it_is_placed() {
    let (store, here, _) = two_accounts();
    // The reply leaves from the message's account: give its identity a default for replies.
    let here_reply = rich(
        here,
        "Brief",
        "A.",
        IsDefault::Alternate,
        IsDefault::Default,
    );
    store.put_signature(&here_reply).unwrap();
    // (placement, whether the signature comes before the attribution line)
    for (placement, above) in [
        (Placement::BelowQuote, false),
        (Placement::AboveQuote, true),
    ] {
        let draft = draft_reply_placed(
            &store,
            first_message(),
            ReplyScope::Sender,
            "Yes please",
            at(1),
            placement,
        )
        .unwrap();
        let text = &draft.text;
        let separator = text.find("\r\n-- \r\nA.\r\n").expect(text);
        let quote = text.find("wrote:").expect(text);
        assert!(!text.contains("Ada at Work"), "{placement:?}: {text:?}");
        assert_eq!(separator < quote, above, "{placement:?}: {text:?}");
        assert!(text.starts_with("Yes please"), "{placement:?}: {text:?}");
    }
}

fn para(text: &str) -> Node {
    Node::plain(ParaKind::Paragraph, text)
}

/// Words typed under a signature that is one of the identity's survive its being swapped; a
/// document with no signature takes one at its end, after the line it opens on.
#[test]
fn a_swap_replaces_only_the_signature_it_recognises() {
    let (store, here, there) = two_accounts();
    let work = &store.signatures(here).unwrap()[0];
    let home = &store.signatures(there).unwrap()[0];
    let doc = Doc {
        nodes: vec![
            para("Hello"),
            Node::Object(mail_editor::Object::Signature),
            para("Ada at Work"),
            para("PS: typed under it"),
        ],
    };
    assert_eq!(
        recognise(&doc, std::slice::from_ref(work)),
        Some((work.id, 1..3))
    );
    let (op, at) = swap(
        &doc,
        std::slice::from_ref(work),
        Some(home),
        Placement::default(),
    )
    .expect("a different signature is an edit");
    assert_eq!(at, 1);
    let mut swapped = doc.clone();
    apply(&mut swapped, op).unwrap();
    let words: Vec<String> = swapped
        .nodes
        .iter()
        .map(|n| plain_of(std::slice::from_ref(n)))
        .collect();
    assert_eq!(words, ["Hello", "", "Ada at Home", "PS: typed under it"]);
    assert_eq!(
        swap(
            &swapped,
            std::slice::from_ref(home),
            Some(home),
            Placement::default()
        ),
        None,
        "the same signature again is no edit"
    );

    let blank = Doc {
        nodes: vec![para("")],
    };
    let (op, at) = swap(&blank, &[], Some(home), Placement::default()).unwrap();
    assert_eq!(at, 1, "the line it opens on stays above the signature");
    let mut signed = blank.clone();
    apply(&mut signed, op).unwrap();
    assert_eq!(region(&signed), Some(1..3));
}

/// A picture put in a signature's words is left out of what is kept: its `cid:` names a part
/// only the page it was put on carries, and every message signed with it would show it broken.
#[test]
fn a_signature_keeps_its_words_and_leaves_out_a_picture() {
    let (store, here, _) = two_accounts();
    let work = &store.signatures(here).unwrap()[0];
    let doc = Doc {
        nodes: vec![
            para("Ada at Work"),
            Node::Object(mail_editor::Object::Image {
                src: mail_editor::ImageRef::of_cid("logo@example.test"),
                alt: "Logo".to_owned(),
            }),
        ],
    };
    let kept = write(&store, work.id, &doc).unwrap();
    let html = kept.html.as_deref().expect("words keep their HTML");
    assert!(html.contains("Ada at Work"), "{html}");
    assert!(!html.contains("logo@example.test"), "{html}");
    assert!(!html.contains("<img"), "{html}");
    assert_eq!(store.signature(work.id).unwrap(), kept);
}

/// A deleted signature is handed back whole, and putting it back gives the identity the same
/// signature again: its id, words and defaults.
#[test]
fn a_deleted_signature_is_put_back_as_it_was() {
    let (store, here, _) = two_accounts();
    let work = store.signatures(here).unwrap().remove(0);
    let had = delete(&store, work.id).unwrap();
    assert_eq!(had, work);
    assert!(store.signatures(here).unwrap().is_empty());
    restore(&store, &had).unwrap();
    assert_eq!(store.signatures(here).unwrap(), std::slice::from_ref(&work));
    assert_eq!(
        default_for(&store, here, SignatureUse::Replies).unwrap(),
        Some(work)
    );
}
