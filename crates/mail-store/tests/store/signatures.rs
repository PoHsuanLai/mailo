//! Signatures: several to an identity, one the default for new mail and one for replies, and
//! `set_signature`, the single one the command line speaks of, kept on top of them.

use mail_domain::id::new_account_id;
use mail_domain::{IdentityId, IsDefault, Signature, SignatureId, SignatureUse};
use mail_store::StoreError;
use mail_store::testing::{in_memory, seed_account, seed_default_identity};

fn with_identity() -> (mail_store::SqliteStore, IdentityId) {
    let store = in_memory();
    let account = new_account_id();
    seed_account(&store, account.clone(), "ada@example.test");
    let identity = seed_default_identity(&store, account, "ada@example.test", Some("Ada"));
    (store, identity)
}

fn signature(
    identity: IdentityId,
    name: &str,
    new_mail: IsDefault,
    replies: IsDefault,
) -> Signature {
    Signature {
        id: SignatureId::generate(),
        identity,
        name: name.to_owned(),
        html: Some(format!("<p><b>{name}</b></p>")),
        text: name.to_owned(),
        new_mail,
        replies,
    }
}

/// The ids of the identity's defaults, for new mail and for replies.
fn defaults(store: &mail_store::SqliteStore, identity: IdentityId) -> [Option<SignatureId>; 2] {
    [SignatureUse::NewMail, SignatureUse::Replies].map(|purpose| {
        store
            .default_signature(identity, purpose)
            .unwrap()
            .map(|s| s.id)
    })
}

#[test]
fn signatures_are_listed_in_the_order_they_were_made_and_rewritten_in_place() {
    let (store, mine) = with_identity();
    let long = signature(mine, "Long", IsDefault::Default, IsDefault::Alternate);
    let short = signature(mine, "Short", IsDefault::Alternate, IsDefault::Default);
    store.put_signature(&long).unwrap();
    store.put_signature(&short).unwrap();
    let renamed = Signature {
        name: "Formal".to_owned(),
        ..long.clone()
    };
    store.put_signature(&renamed).unwrap();

    assert_eq!(
        store.signatures(mine).unwrap(),
        vec![renamed.clone(), short.clone()]
    );
    assert_eq!(store.signature(short.id).unwrap(), short);
    assert_eq!(defaults(&store, mine), [Some(long.id), Some(short.id)]);
}

#[test]
fn a_new_default_takes_the_flag_from_the_one_that_had_it() {
    let (store, mine) = with_identity();
    let first = signature(mine, "First", IsDefault::Default, IsDefault::Default);
    store.put_signature(&first).unwrap();
    assert_eq!(defaults(&store, mine), [Some(first.id), Some(first.id)]);

    let second = signature(mine, "Second", IsDefault::Alternate, IsDefault::Default);
    store.put_signature(&second).unwrap();

    assert_eq!(defaults(&store, mine), [Some(first.id), Some(second.id)]);
    assert_eq!(
        store.signature(first.id).unwrap().replies,
        IsDefault::Alternate,
        "two defaults for replies"
    );
}

#[test]
fn a_deleted_signature_is_gone() {
    let (store, mine) = with_identity();
    let one = signature(mine, "One", IsDefault::Default, IsDefault::Default);
    store.put_signature(&one).unwrap();
    store.delete_signature(one.id).unwrap();
    assert_eq!(store.signatures(mine).unwrap(), vec![]);
    assert!(matches!(
        store.delete_signature(one.id),
        Err(StoreError::NoSignature(id)) if id == one.id
    ));
    assert!(matches!(
        store.signature(one.id),
        Err(StoreError::NoSignature(_))
    ));
}

/// `set_signature` speaks of the default for new mail: it rewrites that one's words (and drops
/// the HTML that said something else), makes one when there is none, and deletes it with `None`,
/// leaving the identity's other signatures as they were.
#[test]
fn set_signature_writes_the_default_for_new_mail_and_leaves_the_rest() {
    let (store, mine) = with_identity();
    store.set_signature(mine, Some("Ada")).unwrap();
    let made = store.signatures(mine).unwrap();
    assert_eq!(made.len(), 1);
    assert_eq!(
        (made[0].new_mail, made[0].replies, made[0].html.as_deref()),
        (IsDefault::Default, IsDefault::Default, None)
    );

    let other = signature(mine, "Other", IsDefault::Alternate, IsDefault::Default);
    store.put_signature(&other).unwrap();
    let rich = Signature {
        html: Some("<p><i>Ada</i></p>".to_owned()),
        ..store.signature(made[0].id).unwrap()
    };
    store.put_signature(&rich).unwrap();
    store.set_signature(mine, Some("Ada L.")).unwrap();
    let rewritten = store.signature(made[0].id).unwrap();
    assert_eq!((rewritten.text.as_str(), rewritten.html), ("Ada L.", None));
    assert_eq!(
        store.identity(mine).unwrap().unwrap().signature.as_deref(),
        Some("Ada L.")
    );

    store.set_signature(mine, None).unwrap();
    assert_eq!(store.signatures(mine).unwrap(), vec![other]);
    assert_eq!(store.identity(mine).unwrap().unwrap().signature, None);
}
