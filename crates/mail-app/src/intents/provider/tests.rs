//! The provider over a store, with no bus: what an action does, what it hands back to undo it,
//! and what it labels as somebody else's words.

use super::*;
use crate::intents::wire::{Integrity, Invocation, Label, Output, Target};
use chrono::{TimeZone, Utc};
use mail_domain::id::{account_id_from_uuid, new_account_id};
use mail_domain::*;
use mail_runtime::MapSigningStore;
use mail_store::Store;
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

fn thread_of(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
}

fn message(store: &SqliteStore, n: u128, subject: &str, body: &str) -> Message {
    let raw = store.blobs().put(body.as_bytes()).expect("blob");
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        thread: thread_of(n),
        account: acct_account(),
        key: MessageKey::Rfc(format!("m{n}@b.c")),
        date: Utc
            .timestamp_opt(1_700_000_000 + n as i64 * 60, 0)
            .single()
            .expect("time"),
        from: Address {
            name: Some("Ada".to_owned()),
            email: "ada@b.c".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@b.c")),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some(body.to_owned()),
            raw,
        },
        attachments: vec![],
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::Supported,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::DropInbox,
        folders: FolderRoles::default(),
        condstore: Condstore::Supported,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Yes,
        connections: ConnectionBudget::default(),
        observed_at: Utc::now(),
    }
}

/// A store with one account that can send, and three conversations in its inbox.
pub(crate) fn world() -> (Provider, Arc<SqliteStore>, tempfile::TempDir) {
    world_opening(Opener::new(|_, _| Ok(())))
}

/// The same, opening conversations with `opener`.
fn world_opening(opener: Opener) -> (Provider, Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(SqliteStore::in_memory(dir.path()).expect("sqlite"));
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual("me@example.test", &manual, Utc::now());
    {
        mail_store::testing::seed_account_plan(
            &store,
            acct_account(),
            &preset.plan.address,
            &preset.plan,
            None,
        );
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            "me@example.test",
            None,
        );
        mail_store::testing::seed_caps(&store, acct_account(), &caps(), chrono::Utc::now())
            .unwrap();
    }
    for (n, subject, body) in [
        (1, "Lunch on Friday", "Shall we eat at noon?"),
        (2, "Quarterly report", "The numbers are attached."),
        (3, "Lunch menu", "Soup and bread."),
    ] {
        let upsert = Change::MessageUpsert(Box::new(message(&store, n, subject, body)));
        store
            .apply(
                acct_account(),
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![upsert],
                },
            )
            .expect("apply");
        // Where the server keeps it, so that what is done here has a server half to queue.
        mail_store::testing::seed_remote_uid(
            &store,
            acct_account(),
            "INBOX",
            n as u32,
            MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        );
    }
    let provider = Provider::new(store.clone(), Arc::new(MapSigningStore::default()), opener);
    (provider, store, dir)
}

fn id(kind: &str, key: impl std::fmt::Display) -> EntityId {
    EntityId {
        app: APP.to_owned(),
        kind: kind.to_owned(),
        key: key.to_string(),
    }
}

fn threads(ns: &[u128]) -> Target {
    Target::Entities(
        ns.iter()
            .map(|n| id("mail.thread", thread_of(*n).as_uuid()))
            .collect(),
    )
}

/// The router's label for what the person typed.
fn typed() -> serde_json::Value {
    serde_json::json!({ "integrity": "trusted", "confidentiality": { "kind": "public" },
                        "classes": [], "sources": [{ "kind": "user" }] })
}

/// The router's label for what an agent took from a message.
fn lifted() -> serde_json::Value {
    serde_json::json!({ "integrity": "untrusted", "confidentiality": { "kind": "private", "v": ["work"] },
                        "classes": ["mail"], "sources": [{ "kind": "mail" }, { "kind": "model", "v": "planner" }] })
}

/// An invocation as the router sends it: `args` are `(name, value as JSON)`, each typed by the
/// person.
fn call(action: &str, target: Target, args: &[(&str, serde_json::Value)]) -> Invocation {
    let labelled: Vec<_> = args
        .iter()
        .map(|(name, value)| (*name, value.clone(), typed()))
        .collect();
    call_labelled(action, target, &labelled)
}

/// The same, each argument with the label given.
fn call_labelled(
    action: &str,
    target: Target,
    args: &[(&str, serde_json::Value, serde_json::Value)],
) -> Invocation {
    let args: serde_json::Map<String, serde_json::Value> = args
        .iter()
        .map(|(name, value, label)| {
            (
                (*name).to_owned(),
                serde_json::json!({ "value": value, "label": label }),
            )
        })
        .collect();
    let target = match target {
        Target::Entities(ids) => {
            serde_json::json!({ "kind": "entities", "v": ids.iter().map(|i| serde_json::json!({"app": i.app, "kind": i.kind, "key": i.key})).collect::<Vec<_>>() })
        }
        _ => serde_json::json!({ "kind": "nothing" }),
    };
    serde_json::from_value(serde_json::json!({
        "call": 1, "action": action, "target": target, "args": args, "space": "work",
    }))
    .expect("an invocation")
}

fn text(value: &str) -> serde_json::Value {
    serde_json::json!({ "kind": "text", "v": value })
}

fn token_of(outcome: &Outcome) -> String {
    match &outcome.undo {
        Undoable::Yes(token) => token.clone(),
        Undoable::No => panic!("no undo token: {outcome:?}"),
    }
}

fn summary(store: &SqliteStore, n: u128) -> ThreadSummary {
    store.thread(thread_of(n)).expect("thread").summary
}

#[test]
fn archiving_takes_the_conversation_out_of_the_inbox_and_the_token_puts_it_back() {
    let (provider, store, _dir) = world();
    let outcome = provider
        .perform(&call("mail.thread.archive", threads(&[1]), &[]))
        .expect("archived");
    assert_eq!(outcome.said.as_deref(), Some("Archived"));
    assert!(
        !summary(&store, 1).mailboxes.contains(MailboxRole::Inbox),
        "still in the inbox"
    );
    assert!(
        summary(&store, 2).mailboxes.contains(MailboxRole::Inbox),
        "only the named one moves"
    );
    // The server is told too, as a click tells it.
    let queued = store
        .outbox_due(acct_account(), Utc::now() + chrono::TimeDelta::days(1))
        .expect("outbox");
    assert!(
        queued.iter().any(|entry| matches!(
            entry.op,
            ProtoOp::SetLabels { .. } | ProtoOp::SetMailbox { .. }
        )),
        "nothing was queued for the server: {:?}",
        queued.iter().map(|entry| &entry.op).collect::<Vec<_>>()
    );

    let token = token_of(&outcome);
    assert_eq!(provider.undo(&token), Ok(()));
    assert!(
        summary(&store, 1).mailboxes.contains(MailboxRole::Inbox),
        "not put back"
    );
    assert_eq!(
        provider.undo(&token),
        Err(UndoFault::Gone),
        "a token is used once"
    );
}

/// Make every write to the outbox fail, as a full disk or a locked database would.
fn refuse_queueing(store: &SqliteStore) {
    mail_store::testing::refuse_outbox_writes(store);
}

#[test]
fn an_archive_the_server_cannot_be_told_is_not_done_here_either() {
    // Queued work that failed to be queued is in no outbox: nothing would retry it or say so,
    // and the next sync would put the conversation back in the inbox without a word.
    let (provider, store, _dir) = world();
    refuse_queueing(&store);
    assert!(
        provider
            .perform(&call("mail.thread.archive", threads(&[1]), &[]))
            .is_err(),
        "the archive said it was done"
    );
    assert!(
        summary(&store, 1).mailboxes.contains(MailboxRole::Inbox),
        "archived here, while the server was never told"
    );
}

#[test]
fn an_undo_the_server_cannot_be_told_is_not_done_here_either() {
    let (provider, store, _dir) = world();
    let outcome = provider
        .perform(&call("mail.thread.archive", threads(&[1]), &[]))
        .expect("archived");
    refuse_queueing(&store);
    assert_eq!(provider.undo(&token_of(&outcome)), Err(UndoFault::Conflict));
    assert!(
        !summary(&store, 1).mailboxes.contains(MailboxRole::Inbox),
        "put back here, while the server keeps it archived"
    );
}

#[test]
fn one_gesture_on_several_conversations_is_one_undo() {
    let (provider, store, _dir) = world();
    let outcome = provider
        .perform(&call("mail.thread.star", threads(&[1, 3]), &[]))
        .expect("starred");
    assert_eq!(
        outcome.said.as_deref(),
        Some("Starred \u{b7} 2 conversations")
    );
    assert_eq!(summary(&store, 1).star, Star::Starred);
    assert_eq!(summary(&store, 3).star, Star::Starred);
    assert_eq!(summary(&store, 2).star, Star::Unstarred);
    assert_eq!(provider.undo(&token_of(&outcome)), Ok(()));
    assert_eq!(summary(&store, 1).star, Star::Unstarred);
    assert_eq!(summary(&store, 3).star, Star::Unstarred);
}

#[test]
fn unstarring_is_the_other_way_and_is_undone_the_same() {
    let (provider, store, _dir) = world();
    provider
        .perform(&call("mail.thread.star", threads(&[2]), &[]))
        .expect("starred");
    let outcome = provider
        .perform(&call("mail.thread.unstar", threads(&[2]), &[]))
        .expect("unstarred");
    assert_eq!(summary(&store, 2).star, Star::Unstarred);
    assert_eq!(provider.undo(&token_of(&outcome)), Ok(()));
    assert_eq!(summary(&store, 2).star, Star::Starred);
}

#[test]
fn a_conversation_that_is_gone_refuses_before_anything_changes() {
    let (provider, store, _dir) = world();
    let gone = Target::Entities(vec![
        id("mail.thread", thread_of(1).as_uuid()),
        id("mail.thread", uuid::Uuid::from_u128(0xdead)),
    ]);
    let refused = provider.perform(&call("mail.thread.archive", gone, &[]));
    assert!(
        matches!(refused, Err(AppRefusal::NotFound(_))),
        "{refused:?}"
    );
    assert!(
        summary(&store, 1).mailboxes.contains(MailboxRole::Inbox),
        "the first was changed"
    );

    let not_a_thread = Target::Entities(vec![id("mail.draft", uuid::Uuid::from_u128(1))]);
    let refused = provider.perform(&call("mail.thread.archive", not_a_thread, &[]));
    assert!(
        matches!(refused, Err(AppRefusal::NotFound(_))),
        "{refused:?}"
    );
}

#[test]
fn a_label_is_put_on_and_taken_off_by_name_and_only_when_it_exists() {
    let (provider, store, _dir) = world();
    let label = mail_domain::Label {
        id: LabelId::generate(),
        account: acct_account(),
        name: "Work".to_owned(),
        color: None,
        origin: LabelOrigin::User,
    };
    store
        .apply(
            acct_account(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(label.clone())],
            },
        )
        .expect("label");
    let outcome = provider
        .perform(&call(
            "mail.thread.label",
            threads(&[2]),
            &[("label", text("work"))],
        ))
        .expect("labelled");
    assert!(summary(&store, 2).labels.contains(&label.id));
    assert_eq!(provider.undo(&token_of(&outcome)), Ok(()));
    assert!(!summary(&store, 2).labels.contains(&label.id));

    provider
        .perform(&call(
            "mail.thread.label",
            threads(&[2]),
            &[("label", text("Work"))],
        ))
        .expect("labelled again");
    provider
        .perform(&call(
            "mail.thread.unlabel",
            threads(&[2]),
            &[("label", text("Work"))],
        ))
        .expect("unlabelled");
    assert!(!summary(&store, 2).labels.contains(&label.id));

    let unknown = provider.perform(&call(
        "mail.thread.label",
        threads(&[2]),
        &[("label", text("Nope"))],
    ));
    assert!(matches!(unknown, Err(AppRefusal::Failed(_))), "{unknown:?}");
    let missing = provider.perform(&call("mail.thread.label", threads(&[2]), &[]));
    assert!(
        matches!(missing, Err(AppRefusal::NeedsParam { ref param, .. }) if param == "label"),
        "{missing:?}"
    );
}

#[test]
fn snoozing_sets_the_time_and_undo_brings_the_conversation_back() {
    let (provider, store, _dir) = world();
    let until = Utc::now().timestamp() + 3600;
    let outcome = provider
        .perform(&call(
            "mail.thread.snooze",
            threads(&[1]),
            &[(
                "until",
                serde_json::json!({ "kind": "date_time", "v": until }),
            )],
        ))
        .expect("snoozed");
    assert!(
        matches!(summary(&store, 1).snooze, Snooze::Until(at) if at.timestamp() == until),
        "{:?}",
        summary(&store, 1).snooze
    );
    assert_eq!(provider.undo(&token_of(&outcome)), Ok(()));
    assert_eq!(summary(&store, 1).snooze, Snooze::Inactive);
}

#[test]
fn a_search_finds_conversations_by_mailos_own_language() {
    let (provider, _store, _dir) = world();
    let hits = provider.search("lunch");
    let subjects: Vec<&str> = hits
        .iter()
        .map(|hit| hit.entity.title.value.as_str())
        .collect();
    assert_eq!(hits.len(), 2, "{subjects:?}");
    assert!(subjects.contains(&"Lunch on Friday") && subjects.contains(&"Lunch menu"));
    let hit = &hits[0].entity;
    assert_eq!(hit.id.kind, "mail.thread");
    assert!(hit.id.key.parse::<uuid::Uuid>().is_ok());
    // A subject is somebody else's words, and says so.
    let json = serde_json::to_value(&hit.title).expect("json");
    assert_eq!(json["label"]["integrity"], "untrusted");
    assert_eq!(json["label"]["sources"][0]["kind"], "mail");

    assert!(provider.search("zebra").is_empty());

    let found = provider
        .perform(&call(
            "mail.thread.search",
            Target::Nothing,
            &[("query", text("report"))],
        ))
        .expect("searched");
    assert_eq!(found.said.as_deref(), Some("Found 1 conversation"));
    match found.value.expect("a value").value {
        Output::Entities(ids) => assert_eq!(ids, vec![id("mail.thread", thread_of(2).as_uuid())]),
        other => panic!("{other:?}"),
    }
    assert_eq!(found.undo, Undoable::No);
}

#[test]
fn reading_gives_the_words_as_untrusted_mail_private_to_the_space() {
    let (provider, _store, _dir) = world();
    let outcome = provider
        .perform(&call("mail.thread.read", threads(&[2]), &[]))
        .expect("read");
    let value = outcome.value.expect("a value");
    match &value.value {
        Output::Text(text) => {
            assert!(
                text.contains("Quarterly report") && text.contains("The numbers are attached."),
                "{text}"
            );
            assert!(text.contains("ada@b.c"));
        }
        other => panic!("{other:?}"),
    }
    let label = serde_json::to_value(&value).expect("json")["label"].clone();
    assert_eq!(label["integrity"], "untrusted");
    assert_eq!(
        label["confidentiality"],
        serde_json::json!({ "kind": "private", "v": ["work"] })
    );
    assert_eq!(label["classes"], serde_json::json!(["mail"]));
    assert_eq!(outcome.undo, Undoable::No);
}

/// The launcher's Enter on a mail hit: the conversation goes to the window, and the answer is
/// nothing to show.
#[test]
fn opening_hands_the_conversation_to_the_window_and_answers_nothing() {
    // Each conversation opened, with the token it came with.
    type Opened = Vec<(ThreadId, Option<String>)>;
    let opened: Arc<Mutex<Opened>> = Arc::default();
    let seen = opened.clone();
    let (provider, _store, _dir) = world_opening(Opener::new(move |thread, token| {
        seen.lock()
            .expect("opened")
            .push((thread, token.map(str::to_owned)));
        Ok(())
    }));
    let outcome = provider
        .perform(&call("mail.thread.open", threads(&[2]), &[]))
        .expect("opened");
    assert_eq!(*opened.lock().expect("opened"), [(thread_of(2), None)]);
    assert_eq!(outcome.undo, Undoable::No);
    assert!(
        outcome.value.is_none() && outcome.said.is_none(),
        "{outcome:?}"
    );
    assert_eq!(outcome.follow, crate::intents::wire::Follow::Nothing);

    let gone = provider.perform(&call("mail.thread.open", threads(&[99]), &[]));
    assert!(matches!(gone, Err(AppRefusal::NotFound(_))), "{gone:?}");
    let two = provider.perform(&call("mail.thread.open", threads(&[1, 2]), &[]));
    assert!(matches!(two, Err(AppRefusal::Unsupported)), "{two:?}");
    assert_eq!(
        opened.lock().expect("opened").len(),
        1,
        "a refusal opened something"
    );

    // The launcher's token goes to the window with the conversation.
    let mut launched = call("mail.thread.open", threads(&[3]), &[]);
    launched.activation = Some("launcher-token-1".to_owned());
    provider.perform(&launched).expect("opened");
    assert_eq!(
        opened.lock().expect("opened").last(),
        Some(&(thread_of(3), Some("launcher-token-1".to_owned())))
    );

    let (failing, _store, _dir) =
        world_opening(Opener::new(|_, _| Err("no window here".to_owned())));
    let failed = failing.perform(&call("mail.thread.open", threads(&[1]), &[]));
    assert!(
        matches!(failed, Err(AppRefusal::Failed(ref why)) if why == "no window here"),
        "{failed:?}"
    );
}

#[test]
fn a_preview_is_the_subject_and_the_newest_messages() {
    let (provider, _store, _dir) = world();
    match provider.preview(&id("mail.thread", thread_of(1).as_uuid())) {
        Preview::Thread { subject, messages } => {
            assert_eq!(subject.value, "Lunch on Friday");
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].snippet.value, "Shall we eat at noon?");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        provider.preview(&id("mail.thread", uuid::Uuid::from_u128(9))),
        Preview::None
    );
}

#[test]
fn a_draft_is_saved_and_discarded_by_its_token() {
    let (provider, store, _dir) = world();
    let outcome = provider
        .perform(&call(
            "mail.draft.create",
            Target::Nothing,
            &[
                ("to", text("ada@b.c, Bob <bob@b.c>")),
                ("subject", text("Hello")),
                ("body", text("Hi there")),
            ],
        ))
        .expect("drafted");
    let draft = match outcome.value.as_ref().expect("a value").value.clone() {
        Output::Entities(ids) => ids[0]
            .key
            .parse::<uuid::Uuid>()
            .map(DraftId::from_uuid)
            .expect("a draft id"),
        other => panic!("{other:?}"),
    };
    let stored = store.draft(draft).expect("saved");
    assert_eq!(stored.subject, "Hello");
    assert_eq!(stored.to.len(), 2);
    assert!(stored.text.contains("Hi there"));
    assert_eq!(
        stored.state,
        SendState::Editing,
        "a draft is sent by nobody"
    );

    assert_eq!(provider.undo(&token_of(&outcome)), Ok(()));
    assert!(store.draft(draft).is_err(), "the draft is still there");
    assert_eq!(provider.undo(&token_of(&outcome)), Err(UndoFault::Gone));
}

fn no_drafts(store: &SqliteStore) -> bool {
    mail_store::testing::count(&store, "drafts") == 0
}

fn sent_args() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("to", text("ada@b.c")),
        ("subject", text("Lunch?")),
        ("body", text("Noon on Friday.")),
    ]
}

#[test]
fn a_send_is_queued_and_can_be_taken_back_to_a_draft_until_it_is_delivered() {
    let (provider, store, _dir) = world();
    let outcome = provider
        .perform(&call("mail.message.send", Target::Nothing, &sent_args()))
        .expect("queued");
    assert_eq!(outcome.said.as_deref(), Some("Queued for delivery"));
    let token = token_of(&outcome);
    let draft = match Token::parse(&token) {
        Some(Token::Unsend(drafts)) => match drafts.as_slice() {
            [draft] => *draft,
            more => panic!("{more:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert_eq!(store.draft(draft).expect("kept").state, SendState::Queued);
    assert!(
        store
            .outbox_due(acct_account(), Utc::now() + chrono::TimeDelta::days(1))
            .expect("outbox")
            .iter()
            .any(|entry| matches!(entry.op, ProtoOp::Submit { .. })),
        "nothing waits in the outbox"
    );

    assert_eq!(provider.undo(&token), Ok(()));
    assert_eq!(store.draft(draft).expect("kept").state, SendState::Editing);
}

#[test]
fn what_a_send_would_send_is_shown_before_it_is() {
    let (provider, store, _dir) = world();
    let preview = provider
        .dry_run(&call("mail.message.send", Target::Nothing, &sent_args()))
        .expect("a preview");
    match preview {
        Preview::Message { to, subject, body } => {
            assert_eq!(
                to.iter().map(|t| t.value.as_str()).collect::<Vec<_>>(),
                ["ada@b.c"]
            );
            assert_eq!(subject.value, "Lunch?");
            assert_eq!(body.value, "Noon on Friday.");
        }
        other => panic!("{other:?}"),
    }
    assert!(no_drafts(&store), "a preview made a draft");
}

/// Flow (c): a recipient an agent lifted from a message is shown on the confirmation sheet as
/// somebody else's words, and what the person typed as theirs.
#[test]
fn a_preview_carries_each_arguments_label_on() {
    let (provider, _store, _dir) = world();
    let preview = provider
        .dry_run(&call_labelled(
            "mail.message.send",
            Target::Nothing,
            &[
                ("to", text("ada@b.c, eve@evil.example"), lifted()),
                ("body", text("Noon on Friday."), typed()),
            ],
        ))
        .expect("a preview");
    let Preview::Message { to, subject, body } = preview else {
        panic!("{preview:?}");
    };
    assert_eq!(to.len(), 2);
    for recipient in &to {
        assert_eq!(
            serde_json::to_value(&recipient.label).expect("json"),
            lifted()
        );
    }
    assert_eq!(serde_json::to_value(&body.label).expect("json"), typed());
    assert_eq!(
        subject.label,
        Label::own(APP),
        "no subject was given: mailo's empty one"
    );
}

fn befriend(store: &SqliteStore) {
    store
        .put_contact(
            "accounting@example.test",
            Some("Accounting"),
            &mail_store::Origin::Manual,
        )
        .expect("contact");
}

#[test]
fn a_contact_search_finds_people_in_the_book_and_labels_them_the_persons_own() {
    let (provider, store, _dir) = world();
    befriend(&store);
    let outcome = provider
        .perform(&call(
            "mail.contact.search",
            Target::Nothing,
            &[("query", text("account"))],
        ))
        .expect("searched");
    assert_eq!(outcome.said.as_deref(), Some("Found 1 contact"));
    let found = outcome.value.expect("a value");
    assert_eq!(
        found.value,
        Output::Entities(vec![id("mail.contact", "accounting@example.test")])
    );
    assert_eq!(found.label, Label::contacts("work"));
    assert_eq!(found.label.integrity(), Integrity::Trusted);
    let none = provider
        .perform(&call(
            "mail.contact.search",
            Target::Nothing,
            &[("query", text("zebra"))],
        ))
        .expect("searched");
    assert_eq!(none.value.map(|v| v.value), Some(Output::Entities(vec![])));
}

fn forward_to(contact: &str, label: serde_json::Value, ns: &[u128]) -> Invocation {
    call_labelled(
        "mail.message.forward",
        threads(ns),
        &[(
            "to",
            serde_json::json!({ "kind": "entity", "v": { "app": APP, "kind": "mail.contact", "key": contact } }),
            label,
        )],
    )
}

/// Flow (a): forward two conversations to a contact, and take both back with one token.
#[test]
fn a_forward_queues_one_message_a_conversation_and_one_token_takes_them_all_back() {
    let (provider, store, _dir) = world();
    befriend(&store);
    let outcome = provider
        .perform(&forward_to("accounting@example.test", typed(), &[1, 2]))
        .expect("queued");
    assert_eq!(
        outcome.said.as_deref(),
        Some("2 forwards queued for delivery")
    );
    let token = token_of(&outcome);
    let Some(Token::Unsend(drafts)) = Token::parse(&token) else {
        panic!("{token}");
    };
    assert_eq!(drafts.len(), 2);
    let subjects: Vec<String> = drafts
        .iter()
        .map(|draft| {
            let draft = store.draft(*draft).expect("kept");
            assert_eq!(draft.state, SendState::Queued);
            assert_eq!(
                draft
                    .to
                    .iter()
                    .map(|a| a.email.as_str())
                    .collect::<Vec<_>>(),
                ["accounting@example.test"]
            );
            draft.subject
        })
        .collect();
    assert_eq!(subjects, ["Fwd: Lunch on Friday", "Fwd: Quarterly report"]);

    assert_eq!(provider.undo(&token), Ok(()));
    for draft in &drafts {
        assert_eq!(store.draft(*draft).expect("kept").state, SendState::Editing);
    }
    assert_eq!(
        provider.undo(&token),
        Err(UndoFault::Gone),
        "taken back once"
    );
}

#[test]
fn what_a_forward_would_send_is_shown_with_the_mail_labelled_as_mail() {
    let (provider, store, _dir) = world();
    befriend(&store);
    let preview = provider
        .dry_run(&forward_to("accounting@example.test", lifted(), &[2]))
        .expect("a preview");
    let Preview::Message { to, subject, body } = preview else {
        panic!("{preview:?}");
    };
    assert_eq!(
        to.iter().map(|t| t.value.as_str()).collect::<Vec<_>>(),
        ["Accounting <accounting@example.test>"]
    );
    // The book's address, chosen by words lifted from mail: the worse of the two.
    assert_eq!(to[0].label.integrity(), Integrity::Untrusted);
    let to_label = serde_json::to_value(&to[0].label).expect("json");
    assert!(
        to_label["sources"]
            .as_array()
            .expect("sources")
            .contains(&serde_json::json!({ "kind": "contacts" }))
    );
    assert_eq!(subject.value, "Fwd: Quarterly report");
    assert_eq!(subject.label, Label::mail("work"));
    assert!(
        body.value.contains("The numbers are attached."),
        "the forwarded words are shown"
    );
    assert_eq!(body.label, Label::mail("work"));
    assert!(no_drafts(&store), "a preview made a draft");
}

#[test]
fn a_forward_to_someone_not_in_the_book_or_of_nothing_refuses_and_leaves_nothing() {
    let (provider, store, _dir) = world();
    befriend(&store);
    let stranger = provider.perform(&forward_to("eve@evil.example", typed(), &[1]));
    assert!(
        matches!(stranger, Err(AppRefusal::NotFound(ref id)) if id.key == "eve@evil.example"),
        "{stranger:?}"
    );
    let gone = provider.perform(&forward_to("accounting@example.test", typed(), &[1, 99]));
    assert!(matches!(gone, Err(AppRefusal::NotFound(_))), "{gone:?}");
    let typed_address = provider.perform(&call(
        "mail.message.forward",
        threads(&[1]),
        &[("to", text("accounting@example.test"))],
    ));
    assert!(
        matches!(typed_address, Err(AppRefusal::NeedsParam { ref param, .. }) if param == "to"),
        "{typed_address:?}"
    );
    assert!(no_drafts(&store), "a refused forward left a draft");
}

#[test]
fn a_token_for_several_messages_reads_back_as_itself() {
    let drafts = vec![DraftId::generate(), DraftId::generate()];
    let token = Token::Unsend(drafts.clone()).to_string();
    assert_eq!(Token::parse(&token), Some(Token::Unsend(drafts)));
    assert_eq!(Token::parse("unsend-"), None);
    assert_eq!(Token::parse("unsend-x.y"), None);
}

#[test]
fn a_send_without_a_recipient_or_a_body_asks_for_them_and_leaves_nothing() {
    let (provider, store, _dir) = world();
    for (args, wanted) in [
        (vec![("body", text("Hi"))], "to"),
        (vec![("to", text("ada@b.c"))], "body"),
    ] {
        let refused = provider.perform(&call("mail.message.send", Target::Nothing, &args));
        assert!(
            matches!(refused, Err(AppRefusal::NeedsParam { ref param, .. }) if param == wanted),
            "{refused:?}"
        );
    }
    let bad = provider.perform(&call(
        "mail.message.send",
        Target::Nothing,
        &[("to", text("not an address")), ("body", text("Hi"))],
    ));
    assert!(matches!(bad, Err(AppRefusal::Failed(_))), "{bad:?}");
    assert!(no_drafts(&store), "a refused send left a draft");
}

#[test]
fn the_sending_account_is_asked_for_when_there_is_a_choice() {
    let (provider, store, _dir) = world();
    let other = new_account_id();
    {
        mail_store::testing::seed_account(&store, other.clone(), "two@example.test");
    }
    let refused = provider.perform(&call("mail.message.send", Target::Nothing, &sent_args()));
    assert!(
        matches!(refused, Err(AppRefusal::NeedsParam { ref param, .. }) if param == "from"),
        "{refused:?}"
    );
    let mut named = sent_args();
    named.push(("from", text("nobody@example.test")));
    let refused = provider.perform(&call("mail.message.send", Target::Nothing, &named));
    assert!(matches!(refused, Err(AppRefusal::Failed(_))), "{refused:?}");
}

#[test]
fn what_is_not_ours_is_refused_in_words_the_router_reads() {
    let (provider, _store, _dir) = world();
    let refused = provider.perform(&call("mail.thread.delete", threads(&[1]), &[]));
    assert_eq!(refused, Err(AppRefusal::Unsupported));
    for token in [
        "",
        "stack-x",
        "stack-999",
        "discard-nope",
        "unsend-",
        "other-1",
        "stack",
    ] {
        assert_eq!(provider.undo(token), Err(UndoFault::Gone), "{token:?}");
    }
    assert_eq!(
        provider.dry_run(&call("mail.thread.archive", threads(&[1]), &[])),
        Ok(Preview::None)
    );
    assert!(
        provider
            .suggest(&SuggestAsk {
                action: crate::intents::wire::ActionRef {
                    app: APP.to_owned(),
                    name: "mail.thread.label".to_owned()
                },
                param: "label".to_owned(),
                typed: String::new(),
            })
            .is_empty()
    );
}

#[test]
fn a_window_less_provider_is_looking_at_nothing() {
    let (provider, _store, _dir) = world();
    let json = serde_json::to_value(provider.context()).expect("json");
    assert_eq!(json["app"], APP);
    assert_eq!(json["here"], serde_json::json!({ "kind": "nowhere" }));
    assert_eq!(json["privacy"], "private");
}

/// The window's opener hands the conversation over from inside the provider's executor, which is
/// a tokio runtime under zbus's `tokio` feature: the blocking handoff must not panic there. No
/// window is running in a test, so whether it was taken does not matter: that it answers does.
/// zbus is a dependency only where the session bus is (not macOS or Windows), as is the handoff.
#[cfg(not(any(target_os = "macos", windows)))]
#[test]
fn the_window_handoff_answers_from_inside_zbus_executor() {
    let _taken: bool =
        zbus::block_on(async { super::handed_to_window(thread_of(1), Some("token")) });
}
