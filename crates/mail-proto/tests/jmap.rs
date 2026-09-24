//! JMAP as values: sessions and responses parsed, requests built, against RFC 8620 and RFC 8621's
//! own examples where they give one and this client's own where they do not.

use mail_domain::{
    AccountId, FolderWork, MailboxRole, NonEmpty, ReadState, SpecialUse, Star, Subscription,
};
use mail_proto::jmap::{self, *};
use mail_proto::{ProtoError, Refusal};
use serde_json::{Value, json};

/// RFC 8620 §2.1's example session resource, as the RFC prints it.
const RFC_SESSION: &str = r#"{
  "capabilities": {
    "urn:ietf:params:jmap:core": {
      "maxSizeUpload": 50000000,
      "maxConcurrentUpload": 8,
      "maxSizeRequest": 10000000,
      "maxConcurrentRequest": 8,
      "maxCallsInRequest": 32,
      "maxObjectsInGet": 256,
      "maxObjectsInSet": 128,
      "collationAlgorithms": [
        "i;ascii-numeric",
        "i;ascii-casemap",
        "i;unicode-casemap"
      ]
    },
    "urn:ietf:params:jmap:mail": {},
    "urn:ietf:params:jmap:contacts": {},
    "https://example.com/apis/foobar": {
      "maxFoosFinangled": 42
    }
  },
  "accounts": {
    "A13824": {
      "name": "john@example.com",
      "isPersonal": true,
      "isReadOnly": false,
      "accountCapabilities": {
        "urn:ietf:params:jmap:mail": {
          "maxMailboxesPerEmail": null,
          "maxMailboxDepth": 10
        },
        "urn:ietf:params:jmap:contacts": {}
      }
    },
    "A97813": {
      "name": "jane@example.com",
      "isPersonal": false,
      "isReadOnly": true,
      "accountCapabilities": {
        "urn:ietf:params:jmap:mail": {
          "maxMailboxesPerEmail": 1,
          "maxMailboxDepth": 10
        }
      }
    }
  },
  "primaryAccounts": {
    "urn:ietf:params:jmap:mail": "A13824",
    "urn:ietf:params:jmap:contacts": "A13824"
  },
  "username": "john@example.com",
  "apiUrl": "https://jmap.example.com/api/",
  "downloadUrl": "https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}",
  "uploadUrl": "https://jmap.example.com/upload/{accountId}/",
  "eventSourceUrl": "https://jmap.example.com/eventsource/?types={types}&closeafter={closeafter}&ping={ping}",
  "state": "75128aab4b1b"
}"#;

#[test]
fn the_rfc_session_names_the_mail_account_and_every_url() {
    let session = Session::parse(RFC_SESSION.as_bytes()).unwrap();
    assert_eq!(session.account, "A13824");
    assert_eq!(session.api_url, "https://jmap.example.com/api/");
    assert_eq!(
        session.download_url,
        "https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}"
    );
    assert_eq!(
        session.upload_url,
        "https://jmap.example.com/upload/{accountId}/"
    );
    assert!(session.event_source_url.is_some());
    assert_eq!(session.limits.max_objects_in_get, 256);
    assert_eq!(session.limits.max_calls_in_request, 32);
    assert_eq!(session.limits.max_size_upload, 50_000_000);
    assert_eq!(session.username, "john@example.com");
    assert_eq!(session.state, "75128aab4b1b");
    // The example's mail account lists no submission capability: it cannot send.
    assert_eq!(session.submission, Submission::Absent);
}

#[test]
fn a_session_with_no_mail_account_is_refused_rather_than_synced_empty() {
    let contacts_only = RFC_SESSION.replace(r#""urn:ietf:params:jmap:mail": "A13824","#, "");
    assert!(matches!(
        Session::parse(contacts_only.as_bytes()),
        Err(ProtoError::Unsupported(_))
    ));
    assert!(matches!(
        Session::parse(b"{\"apiUrl\": 7}"),
        Err(ProtoError::Unsupported(_) | ProtoError::Malformed(_))
    ));
}

#[test]
fn the_download_url_is_expanded_from_the_session_template() {
    let session = Session::parse(RFC_SESSION.as_bytes()).unwrap();
    assert_eq!(
        expand(
            &session.download_url,
            &[
                ("accountId", "A13824"),
                ("blobId", "Gd2f81008"),
                ("type", "message/rfc822"),
                ("name", "message.eml"),
            ]
        ),
        "https://jmap.example.com/download/A13824/Gd2f81008/message.eml?accept=message%2Frfc822"
    );
}

#[test]
fn a_query_and_its_get_go_in_one_request_by_back_reference() {
    let calls = [
        email_query("A1", &["mbDrafts".to_owned()], 0, 50, "q"),
        email_get(
            "A1",
            Ids::ResultOf {
                call: "q".to_owned(),
                name: "Email/query",
                path: "/ids",
            },
            &["id", "keywords"],
            "g",
        ),
    ];
    let golden = json!({
        "using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
        "methodCalls": [
            ["Email/query", {
                "accountId": "A1",
                "filter": { "inMailboxOtherThan": ["mbDrafts"] },
                "sort": [{ "property": "receivedAt", "isAscending": false }],
                "position": 0,
                "limit": 50,
                "calculateTotal": true
            }, "q"],
            ["Email/get", {
                "accountId": "A1",
                "properties": ["id", "keywords"],
                "#ids": { "resultOf": "q", "name": "Email/query", "path": "/ids" }
            }, "g"]
        ]
    });
    assert_eq!(request(&[CORE, MAIL], &calls), golden);
}

#[test]
fn changes_and_mailboxes_are_asked_for_as_the_rfc_spells_them() {
    assert_eq!(
        email_changes("A1", "s41", 256, "c").args,
        json!({ "accountId": "A1", "sinceState": "s41", "maxChanges": 256 })
    );
    assert_eq!(
        mailbox_changes("A1", "m7", "m").args,
        json!({ "accountId": "A1", "sinceState": "m7" })
    );
    assert_eq!(
        mailbox_get("A1", "m").args,
        json!({
            "accountId": "A1",
            "ids": null,
            "properties": ["id", "name", "parentId", "role", "sortOrder", "isSubscribed"]
        })
    );
    // With nothing to leave out there is no filter at all, not an empty one.
    assert!(total_query("A1", &[], "t").args.get("filter").is_none());
}

fn mailboxes() -> Mailboxes {
    Mailboxes::parse(&json!({
        "accountId": "A1",
        "state": "m1",
        "list": [
            { "id": "mbI", "name": "Inbox", "parentId": null, "role": "inbox", "sortOrder": 1, "isSubscribed": true },
            { "id": "mbA", "name": "Archive", "parentId": null, "role": "archive", "sortOrder": 2, "isSubscribed": true },
            { "id": "mbD", "name": "Drafts", "parentId": null, "role": "drafts", "sortOrder": 3, "isSubscribed": true },
            { "id": "mbS", "name": "Sent", "parentId": null, "role": "sent", "sortOrder": 4, "isSubscribed": true },
            { "id": "mbT", "name": "Trash", "parentId": null, "role": "trash", "sortOrder": 5, "isSubscribed": true },
            { "id": "mbJ", "name": "Junk", "parentId": null, "role": "junk", "sortOrder": 6, "isSubscribed": true },
            { "id": "mbW", "name": "Work", "parentId": null, "role": null, "sortOrder": 10, "isSubscribed": true },
            { "id": "mbP", "name": "2026", "parentId": "mbW", "role": null, "sortOrder": 11, "isSubscribed": false }
        ],
        "notFound": []
    }))
    .unwrap()
}

#[test]
fn mailboxes_become_folders_with_paths_and_roles() {
    let m = mailboxes();
    assert_eq!(m.state, "m1");
    assert_eq!(m.path("mbP").as_deref(), Some("Work/2026"));
    assert_eq!(m.id_for_path("Work/2026"), Some("mbP"));
    assert_eq!(m.id_for_role(MailboxRole::Spam), Some("mbJ"));
    assert_eq!(m.unfollowed(), vec!["mbD".to_owned(), "mbJ".to_owned()]);
    let roles = m.roles();
    assert_eq!(roles.path(MailboxRole::Sent), Some("Sent"));
    assert_eq!(roles.path(MailboxRole::Archive), Some("Archive"));
    let folders = m.folders(AccountId::generate());
    let nested = folders.iter().find(|f| f.path == "Work/2026").unwrap();
    assert_eq!(nested.subscription, Subscription::Unsubscribed);
    assert_eq!(nested.delimiter, Some('/'));
    let junk = folders.iter().find(|f| f.path == "Junk").unwrap();
    assert_eq!(junk.special, Some(SpecialUse::Junk));
}

#[test]
fn a_loop_of_parents_ends_the_walk_not_the_program() {
    let m = Mailboxes::parse(&json!({
        "state": "x",
        "list": [
            { "id": "a", "name": "A", "parentId": "b" },
            { "id": "b", "name": "B", "parentId": "a" }
        ]
    }))
    .unwrap();
    assert!(m.path("a").is_some());
}

#[test]
fn an_email_is_filed_by_the_mailboxes_it_is_in() {
    let m = mailboxes();
    let ids = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let cases: &[(&[&str], Filing)] = &[
        (
            &["mbI"],
            Filing::Filed {
                role: MailboxRole::Inbox,
                labels: vec![],
            },
        ),
        (
            &["mbI", "mbW"],
            Filing::Filed {
                role: MailboxRole::Inbox,
                labels: vec!["Work".to_owned()],
            },
        ),
        // Only in a label: kept, out of the inbox.
        (
            &["mbP"],
            Filing::Filed {
                role: MailboxRole::Archive,
                labels: vec!["Work/2026".to_owned()],
            },
        ),
        // Trashed by a client that kept the other mailboxes: in the bin, not the inbox.
        (
            &["mbT", "mbI"],
            Filing::Filed {
                role: MailboxRole::Trash,
                labels: vec![],
            },
        ),
        // Sent to oneself.
        (
            &["mbS", "mbI"],
            Filing::Filed {
                role: MailboxRole::Inbox,
                labels: vec![],
            },
        ),
        // Archived after sending.
        (
            &["mbS", "mbA"],
            Filing::Filed {
                role: MailboxRole::Archive,
                labels: vec![],
            },
        ),
        (
            &["mbS"],
            Filing::Filed {
                role: MailboxRole::Sent,
                labels: vec![],
            },
        ),
        (&["mbD"], Filing::Unfollowed),
        (&["mbJ"], Filing::Unfollowed),
        (&["unknown"], Filing::Unfollowed),
    ];
    for (mailbox_ids, want) in cases {
        assert_eq!(&filing(&ids(mailbox_ids), &m), want, "{mailbox_ids:?}");
    }
}

/// An `Email/get` answer in the shape RFC 8621 §4.9 shows, with the header fields raw.
fn email_answer() -> Value {
    json!({
        "accountId": "A1",
        "state": "e41",
        "list": [{
            "id": "Mf40b5f831",
            "blobId": "Gd2f81008",
            "threadId": "Td957e72e8",
            "mailboxIds": { "mbI": true },
            "keywords": { "$Seen": true, "$flagged": true, "$draft": false },
            "size": 1502,
            "receivedAt": "2026-09-01T09:12:01Z",
            "messageId": ["abc@example.com"],
            "inReplyTo": null,
            "references": null,
            "from": [{ "name": "Joe Bloggs", "email": "joe@example.com" }],
            "to": [{ "name": null, "email": "john@example.com" }],
            "cc": null,
            "bcc": null,
            "replyTo": null,
            "subject": "Dinner on Thursday?",
            "sentAt": "2026-09-01T11:12:00+02:00",
            "hasAttachment": false,
            "preview": "Dear John, would you like to…",
            "headers": [
                { "name": "From", "value": " Joe Bloggs <joe@example.com>" },
                { "name": "To", "value": " john@example.com" },
                { "name": "Subject", "value": " Dinner on\r\n Thursday?" },
                { "name": "Message-ID", "value": " <abc@example.com>" }
            ]
        }],
        "notFound": ["Mmissing"]
    })
}

#[test]
fn an_email_summary_parses_and_rebuilds_its_own_header_block() {
    let emails = EmailSummary::parse_list(&email_answer()).unwrap();
    let email = &emails[0];
    assert_eq!(email.id, "Mf40b5f831");
    assert_eq!(email.blob_id, "Gd2f81008");
    assert_eq!(email.mailbox_ids, vec!["mbI".to_owned()]);
    // Keywords compare without case, and `false` is not membership.
    assert_eq!(email.read(), ReadState::Read);
    assert_eq!(email.star(), Star::Starred);
    assert!(!email.keywords.iter().any(|k| k == "$draft"));
    assert_eq!(email.from[0].email, "joe@example.com");
    assert_eq!(email.message_id, vec!["abc@example.com".to_owned()]);
    assert!(email.in_reply_to.is_empty());
    assert_eq!(email.has_attachment, HasAttachment::No);
    assert_eq!(
        email.raw_headers(),
        b"From: Joe Bloggs <joe@example.com>\r\nTo: john@example.com\r\n\
          Subject: Dinner on\r\n Thursday?\r\nMessage-ID: <abc@example.com>\r\n\r\n"
            .to_vec()
    );
    assert_eq!(state_of(&email_answer()).unwrap(), "e41");
}

#[test]
fn a_field_of_the_wrong_type_is_malformed_not_a_panic() {
    for bad in [
        json!({ "list": [{ "id": 7 }] }),
        json!({ "list": [{ "id": "M1", "keywords": ["$seen"] }] }),
        json!({ "list": [{ "id": "M1", "from": "joe" }] }),
        json!({ "nolist": [] }),
    ] {
        assert!(
            matches!(
                EmailSummary::parse_list(&bad),
                Err(ProtoError::Malformed(_))
            ),
            "{bad}"
        );
    }
}

#[test]
fn changes_and_query_pages_parse() {
    // RFC 8620 §5.2's shape.
    let changes = Changes::parse(&json!({
        "accountId": "A1",
        "oldState": "e41",
        "newState": "e42",
        "hasMoreChanges": true,
        "created": ["M1"],
        "updated": ["M2", "M3"],
        "destroyed": []
    }))
    .unwrap();
    assert_eq!(changes.new_state, "e42");
    assert_eq!(changes.more, More::Yes);
    assert_eq!(changes.updated.len(), 2);
    assert!(!changes.is_empty());
    let page = QueryPage::parse(&json!({
        "accountId": "A1",
        "queryState": "q1",
        "canCalculateChanges": false,
        "position": 50,
        "total": 120,
        "ids": ["M9", "M8"]
    }))
    .unwrap();
    assert_eq!(
        (page.position, page.total, page.ids.len()),
        (50, Some(120), 2)
    );
}

#[test]
fn patches_name_only_what_changed() {
    let m = mailboxes();
    assert_eq!(
        Value::Object(flags_patch(Some(ReadState::Read), Some(Star::Unstarred))),
        json!({ "keywords/$seen": true, "keywords/$flagged": null })
    );
    assert_eq!(
        Value::Object(flags_patch(Some(ReadState::Unread), None)),
        json!({ "keywords/$seen": null })
    );
    // Archive: into Archive, out of Inbox, Trash and Junk; Sent and labels untouched.
    assert_eq!(
        Value::Object(filing_patch(MailboxRole::Archive, &m).unwrap()),
        json!({
            "mailboxIds/mbA": true,
            "mailboxIds/mbI": null,
            "mailboxIds/mbT": null,
            "mailboxIds/mbJ": null
        })
    );
    assert_eq!(
        Value::Object(labels_patch(&["Work/2026".to_owned()], &["Work".to_owned()], &m).unwrap()),
        json!({ "mailboxIds/mbP": true, "mailboxIds/mbW": null })
    );
    assert_eq!(
        Value::Object(keyword_patch(mail_domain::Keyword::MdnSent)),
        json!({ "keywords/$MDNSent": true })
    );
}

#[test]
fn archiving_with_no_archive_mailbox_only_leaves_the_inbox() {
    let m = Mailboxes::parse(&json!({
        "state": "m",
        "list": [{ "id": "mbI", "name": "Inbox", "role": "inbox" }]
    }))
    .unwrap();
    assert_eq!(
        Value::Object(filing_patch(MailboxRole::Archive, &m).unwrap()),
        json!({ "mailboxIds/mbI": null })
    );
    // Trashing where there is no Trash is refused, not guessed at.
    assert!(filing_patch(MailboxRole::Trash, &m).is_err());
    // So is adding a label no mailbox answers to.
    assert!(labels_patch(&["Nowhere".to_owned()], &[], &m).is_err());
}

#[test]
fn folder_work_is_a_mailbox_set_that_never_removes_mail() {
    let m = mailboxes();
    let delete = folder_work(
        &FolderWork::Delete {
            path: "Work/2026".to_owned(),
            non_empty: NonEmpty::Refuse,
        },
        &m,
    )
    .unwrap();
    assert_eq!(
        Value::Object(delete),
        json!({ "destroy": ["mbP"], "onDestroyRemoveEmails": false })
    );
    let create = folder_work(
        &FolderWork::Create {
            path: "Work/Q3".to_owned(),
        },
        &m,
    )
    .unwrap();
    assert_eq!(
        Value::Object(create),
        json!({ "create": { "new": { "name": "Q3", "parentId": "mbW", "isSubscribed": true } } })
    );
    let rename = folder_work(
        &FolderWork::Rename {
            from: "Work/2026".to_owned(),
            to: "Old 2026".to_owned(),
        },
        &m,
    )
    .unwrap();
    assert_eq!(
        Value::Object(rename),
        json!({ "update": { "mbP": { "name": "Old 2026", "parentId": null } } })
    );
}

#[test]
fn a_send_is_an_import_and_a_submission_with_the_envelope_named() {
    let identity = Identity {
        id: "I1".to_owned(),
        email: "john@example.com".to_owned(),
        name: None,
    };
    let calls = submission(
        "A1",
        "Gblob",
        &identity,
        "john@example.com",
        &[
            "ada@example.test".to_owned(),
            "blind@example.test".to_owned(),
        ],
        &Filed {
            drafts: Some("mbD".to_owned()),
            sent: Some("mbS".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(
        request(&[CORE, MAIL, SUBMISSION], &calls)["methodCalls"],
        json!([
            ["Email/import", {
                "accountId": "A1",
                "emails": { "draft": {
                    "blobId": "Gblob",
                    "mailboxIds": { "mbD": true },
                    "keywords": { "$draft": true, "$seen": true }
                } }
            }, "import"],
            ["EmailSubmission/set", {
                "accountId": "A1",
                "create": { "send": {
                    "identityId": "I1",
                    "emailId": "#draft",
                    "envelope": {
                        "mailFrom": { "email": "john@example.com", "parameters": null },
                        "rcptTo": [
                            { "email": "ada@example.test", "parameters": null },
                            { "email": "blind@example.test", "parameters": null }
                        ]
                    }
                } },
                "onSuccessUpdateEmail": { "#send": {
                    "keywords/$draft": null,
                    "mailboxIds/mbD": null,
                    "mailboxIds/mbS": true
                } }
            }, "send"]
        ])
    );
    // Nowhere to file it: refused before anything is sent.
    assert!(
        submission(
            "A1",
            "G",
            &identity,
            "john@example.com",
            &[],
            &Filed {
                drafts: None,
                sent: None
            }
        )
        .is_err()
    );
}

#[test]
fn a_submission_answer_says_what_was_sent_or_why_not() {
    let ok = Responses::parse(
        br#"{"methodResponses":[
            ["Email/import",{"accountId":"A1","created":{"draft":{"id":"M77","blobId":"G","threadId":"T","size":9}}},"import"],
            ["EmailSubmission/set",{"accountId":"A1","created":{"send":{"id":"S1"}}},"send"],
            ["Email/set",{"accountId":"A1","updated":{"M77":null}},"send"]
        ]}"#,
    )
    .unwrap();
    assert_eq!(
        submitted(&ok).unwrap(),
        Sent {
            email_id: "M77".to_owned(),
            submission_id: "S1".to_owned()
        }
    );
    let refused = Responses::parse(
        br#"{"methodResponses":[
            ["Email/import",{"accountId":"A1","created":{"draft":{"id":"M77"}}},"import"],
            ["EmailSubmission/set",{"accountId":"A1","notCreated":{"send":{"type":"forbiddenFrom","description":"not yours"}}},"send"]
        ]}"#,
    )
    .unwrap();
    assert!(matches!(
        submitted(&refused),
        Err(ProtoError::Refused { kind: Refusal::Permanent, ref text }) if text.contains("forbiddenFrom")
    ));
    let limited = Responses::parse(
        br#"{"methodResponses":[
            ["Email/import",{"accountId":"A1","created":{"draft":{"id":"M77"}}},"import"],
            ["EmailSubmission/set",{"accountId":"A1","notCreated":{"send":{"type":"rateLimit"}}},"send"]
        ]}"#,
    )
    .unwrap();
    assert!(matches!(
        submitted(&limited),
        Err(ProtoError::Throttled { .. })
    ));
}

#[test]
fn the_identity_is_the_sender_or_a_wildcard_for_its_domain_never_another() {
    let identities = Identity::parse_list(&json!({
        "list": [
            { "id": "I1", "email": "john@example.com", "name": "John" },
            { "id": "I2", "email": "*@example.org", "name": "" }
        ]
    }))
    .unwrap();
    assert_eq!(
        choose_identity(&identities, "JOHN@example.com").map(|i| i.id.as_str()),
        Some("I1")
    );
    assert_eq!(
        choose_identity(&identities, "any@Example.org").map(|i| i.id.as_str()),
        Some("I2")
    );
    assert_eq!(choose_identity(&identities, "john@example.net"), None);
    assert_eq!(identities[1].name, None);
}

#[test]
fn method_errors_are_classified_by_type_never_by_prose() {
    let responses = Responses::parse(
        br#"{"methodResponses":[
            ["error",{"type":"serverUnavailable","description":"try later"},"a"],
            ["error",{"type":"invalidArguments"},"b"],
            ["error",{"type":"unknownMethod"},"c"]
        ]}"#,
    )
    .unwrap();
    let error = |id: &str| ProtoError::from(responses.answer(id, "Email/get").unwrap_err());
    assert!(matches!(
        error("a"),
        ProtoError::Refused {
            kind: Refusal::Transient,
            ..
        }
    ));
    assert!(matches!(
        error("b"),
        ProtoError::Refused {
            kind: Refusal::Permanent,
            ..
        }
    ));
    assert!(matches!(error("c"), ProtoError::Unsupported(_)));
}

#[test]
fn a_set_answer_separates_what_changed_from_what_was_refused() {
    let result = SetResult::parse(&json!({
        "accountId": "A1",
        "oldState": "e1",
        "newState": "e2",
        "updated": { "M1": null },
        "notUpdated": {
            "M2": { "type": "notFound" },
            "M3": { "type": "invalidProperties", "description": "an email must be in a mailbox" }
        },
        "destroyed": ["M4"]
    }))
    .unwrap();
    assert_eq!(result.updated, vec!["M1".to_owned()]);
    assert_eq!(result.destroyed, vec!["M4".to_owned()]);
    // `notFound` is already done; the other one is the refusal to report.
    assert!(matches!(
        result.first_refusal("x"),
        Some(ProtoError::Refused { ref text, .. }) if text.contains("invalidProperties")
    ));
    let _ = jmap::SUBMISSION;
}
