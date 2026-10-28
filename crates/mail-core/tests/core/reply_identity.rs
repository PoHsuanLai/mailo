//! A reply leaves from the address the original reached: mail to an alias is answered from the
//! alias, a forward of it is sent from it too, and mail to the account’s own address from that;
//! a copy to myself follows a draft to the address it is moved to.

use crate::support::{account, now};
use mail_core::identity::{self, Typed};
use mail_core::{Arrival, SqliteStore, Store, absorb, compose};
use mail_domain::presets::{self, Manual};
use mail_domain::*;

const ME: &str = "me@example.test";
const ALIAS: &str = "press@example.test";

/// A store in a scratch directory holding one IMAP account at [`ME`], its own identity named
/// "Me", and an alias at [`ALIAS`] added as Settings adds one.
fn store(dir: &std::path::Path) -> (SqliteStore, Identity) {
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).expect("a store");
    let plan = presets::manual(
        ME,
        &Manual {
            imap_host: "imap.example.test".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.example.test".to_owned(),
            smtp_port: 465,
            login: None,
        },
        now(),
    )
    .plan;
    mail_store::testing::seed_account_plan(&store, account(), ME, &plan, Some(now()));
    mail_store::testing::seed_default_identity(&store, account(), ME, Some("Me"));
    let alias = identity::add_alias(
        &store,
        account(),
        &Typed {
            name: "Me at the Press".to_owned(),
            email: ALIAS.to_owned(),
            reply_to: String::new(),
        },
    )
    .expect("the alias is added");
    (store, alias)
}

/// Deliver a message from Ada to `to`, Cc Bea, and return its id.
fn delivered(store: &SqliteStore, uidl: &str, to: &str) -> MessageId {
    let raw = format!(
        "From: Ada <ada@example.test>\r\nTo: {to}\r\nCc: bea@example.test, {ME}\r\n\
         Subject: The {uidl}\r\nDate: Mon, 5 Oct 2026 09:00:00 +0000\r\n\
         Message-ID: <{uidl}@example.test>\r\n\r\nHello.\r\n"
    );
    absorb(
        store,
        account(),
        MailboxRef {
            account: account(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        now(),
    )
    .expect("the message is absorbed");
    store
        .message_by_rfc_id(account(), &format!("{uidl}@example.test"))
        .expect("the store answers")
        .expect("the message is stored")
}

#[test]
fn a_reply_to_mail_for_an_alias_is_drafted_from_the_alias() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (store, alias) = store(dir.path());
    let own = compose::identity_of(&store, account(), None).expect("the account's own identity");

    let to_alias = delivered(&store, "alias", &format!("Me at the Press <{ALIAS}>"));
    let reply = compose::draft_reply(&store, to_alias, ReplyScope::All, "", now()).unwrap();
    assert_eq!(reply.identity, alias.id, "the reply leaves from the alias");
    assert_eq!(
        store.draft(reply.id).unwrap().identity,
        alias.id,
        "and is saved so"
    );
    // Reply All copies neither of the person's own addresses back to them.
    let everyone: Vec<&str> = reply
        .to
        .iter()
        .chain(&reply.cc)
        .map(|a| a.email.as_str())
        .collect();
    assert_eq!(everyone, ["ada@example.test", "bea@example.test"]);

    let forward = compose::draft_forward(&store, to_alias, &[], "", now()).unwrap();
    assert_eq!(
        forward.identity, alias.id,
        "the forward leaves from the alias"
    );

    // Mail to the account's own address is answered from it, alias or not.
    let to_me = delivered(&store, "own", ME);
    let reply = compose::draft_reply(&store, to_me, ReplyScope::Sender, "", now()).unwrap();
    assert_eq!(reply.identity, own.id);

    // Once the alias is gone its draft is the account's own, and a new reply is too.
    identity::remove_alias(&store, alias.id).unwrap();
    let reply = compose::draft_reply(&store, to_alias, ReplyScope::Sender, "", now()).unwrap();
    assert_eq!(reply.identity, own.id);
}

#[test]
fn the_plan_keeps_a_copy_of_every_alias_and_the_own_address_stays() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (store, alias) = store(dir.path());
    let plan = |store: &SqliteStore| store.account(account()).unwrap().unwrap().plan.unwrap();
    assert_eq!(
        plan(&store).identities,
        store.identities(account()).unwrap()
    );

    let renamed = identity::set_name(&store, account(), "  Ada Lovelace ").unwrap();
    assert_eq!(renamed.from.name.as_deref(), Some("Ada Lovelace"));
    let edited = identity::edit(
        &store,
        alias.id,
        &Typed {
            name: String::new(),
            email: "news@example.test".to_owned(),
            reply_to: "desk@example.test".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(edited.from.name, None, "a blank name is no name");
    assert_eq!(
        edited.reply_to.map(|a| a.email).as_deref(),
        Some("desk@example.test")
    );
    assert_eq!(
        plan(&store).identities,
        store.identities(account()).unwrap()
    );

    // The own address is the one the account signs in as: it is renamed, never readdressed or
    // removed. A typed address that is not one is refused.
    let moved = Typed {
        email: "elsewhere@example.test".to_owned(),
        ..Typed::of(&renamed)
    };
    assert!(matches!(
        identity::edit(&store, renamed.id, &moved),
        Err(mail_core::CoreError::OwnAddressStays)
    ));
    assert!(matches!(
        identity::remove_alias(&store, renamed.id),
        Err(mail_core::CoreError::OwnAddressStays)
    ));
    let nowhere = Typed {
        email: "no-at-sign".to_owned(),
        ..Typed::default()
    };
    assert!(identity::add_alias(&store, account(), &nowhere).is_err());
    assert_eq!(store.identities(account()).unwrap().len(), 2);
}

#[test]
fn a_copy_to_myself_is_added_once_and_follows_the_draft_to_another_address() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (store, alias) = store(dir.path());
    let draft = compose::draft_new(&store, account(), &[], "", "", now()).unwrap();

    let copied = compose::copy_myself(&store, draft.id, compose::Myself::Bcc).unwrap();
    let bcc: Vec<&str> = copied.bcc.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(bcc, [ME]);
    let again = compose::copy_myself(&store, draft.id, compose::Myself::Cc).unwrap();
    assert!(
        again.cc.is_empty(),
        "the address is already on it: {again:?}"
    );

    let moved = compose::move_draft_to_identity(&store, draft.id, alias.id, now()).unwrap();
    assert_eq!(moved.identity, alias.id);
    let bcc: Vec<&str> = moved.bcc.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(
        bcc,
        [ALIAS],
        "the copy goes to the address it now leaves from"
    );
    assert_eq!(store.draft(draft.id).unwrap(), moved, "and is saved so");
}
