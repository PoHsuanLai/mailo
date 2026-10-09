//! Getting a file out of a message, and the file name that comes with it.
//!
//! The name on an attachment is a MIME parameter chosen by whoever sent the message. It is not a
//! fact about the file and it is not a promise about where it belongs: `../../.ssh/
//! authorized_keys` is a well-formed attachment name. Everything here is about the difference
//! between what a message claims and what gets written.

use mail_core::attach;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
    (store, dir)
}

/// A message carrying one attachment with the name a sender chose.
fn with_attachment(store: &SqliteStore, claimed: &str, bytes: &[u8]) -> MessageId {
    let raw = store.blobs().put(b"raw").unwrap();
    let blob = store.blobs().put(bytes).unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: acct_account(),
        key: MessageKey::Rfc(format!("{}@example.test", uuid::Uuid::new_v4())),
        date: chrono::Utc::now(),
        from: Address {
            name: None,
            email: "sender@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "here you go".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("see attached".to_owned()),
            raw,
        },
        attachments: vec![Attachment {
            name: claimed.to_owned(),
            mime: "application/pdf".to_owned(),
            size: bytes.len() as u64,
            content: PartContent::Held(blob),
            inline: Inline::Attached,
        }],
    };
    store
        .ingest(
            acct_account(),
            Ingest {
                mailbox: MailboxRef {
                    account: acct_account(),
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: uuid::Uuid::new_v4().to_string(),
                    },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    id
}

/// A message with nothing attached, which is most of them.
fn plain_message(store: &SqliteStore) -> MessageId {
    let id = with_attachment(store, "x.pdf", b"x");
    // Rewritten through the store rather than constructed separately, so the only difference
    // from the fixture above is the thing being tested.
    mail_store::testing::edit_messages(store, |message| {
        if message.id != id {
            return false;
        }
        message.attachments.clear();
        true
    });
    id
}

mod names {
    use super::*;

    #[test]
    fn a_name_that_climbs_out_of_the_directory_does_not() {
        // The whole reason this function exists. Each of these is a valid MIME filename.
        for hostile in [
            "../../../.ssh/authorized_keys",
            "../../etc/passwd",
            "/etc/passwd",
            "..\\..\\Windows\\System32\\evil.dll",
            "C:\\Users\\ada\\evil.exe",
            "dir/../../../escape.sh",
        ] {
            let safe = attach::safe_name(hostile);
            assert!(
                !safe.contains('/'),
                "{hostile:?} kept a separator: {safe:?}"
            );
            assert!(
                !safe.contains('\\'),
                "{hostile:?} kept a separator: {safe:?}"
            );
            assert!(safe != ".." && safe != ".", "{hostile:?} -> {safe:?}");
            assert!(!safe.is_empty());
        }
    }

    /// `(row, claimed name, name written)`.
    const CASES: &[(&str, &str, &str)] = &[
        // A climbing name keeps the name a person would expect, not mangled beyond recognition.
        ("climbs, unix", "../../etc/passwd", "passwd"),
        ("climbs, windows", "..\\..\\Windows\\evil.dll", "evil.dll"),
        ("absolute", "/etc/passwd", "passwd"),
        // Only dots or nothing gets a name.
        ("empty", "", "attachment"),
        ("blank", "   ", "attachment"),
        ("dot", ".", "attachment"),
        ("dot dot", "..", "attachment"),
        ("slash", "/", "attachment"),
        ("backslash", "\\", "attachment"),
        ("slashes", "///", "attachment"),
        // The rule must not be so eager that it mangles normal mail.
        ("ordinary", "report.pdf", "report.pdf"),
        (
            "spaces and brackets",
            "Q3 results (final).xlsx",
            "Q3 results (final).xlsx",
        ),
        ("not latin", "架構圖.png", "架構圖.png"),
        ("dashes", "notes-2026-09-22.md", "notes-2026-09-22.md"),
        ("dotfile", ".gitignore", ".gitignore"),
    ];

    #[test]
    fn safe_name_cases() {
        for &(row, claimed, want) in CASES {
            assert_eq!(attach::safe_name(claimed), want, "{row}: {claimed:?}");
        }
    }

    #[test]
    fn control_characters_do_not_survive() {
        // A newline in a name makes a terminal print something other than what was written, and
        // a NUL truncates the name at the syscall — so the file that appears is not the file
        // the listing described.
        let sneaky = attach::safe_name("report.pdf\n\rTotal: 0 files\u{0}.exe");
        assert!(
            !sneaky.contains('\n') && !sneaky.contains('\r'),
            "{sneaky:?}"
        );
        assert!(!sneaky.contains('\u{0}'), "{sneaky:?}");
        assert!(sneaky.starts_with("report.pdf"), "{sneaky:?}");
    }

    #[test]
    fn a_very_long_name_is_shortened_but_keeps_what_opens_it() {
        let long = format!("{}.pdf", "a".repeat(400));
        let safe = attach::safe_name(&long);
        assert!(safe.len() <= 255, "{} bytes", safe.len());
        assert!(
            safe.ends_with(".pdf"),
            "the extension decides which program opens it: {safe:?}"
        );
    }
}

mod saving {
    use super::*;

    #[test]
    fn an_attachment_is_written_where_it_was_asked_for_and_nowhere_else() {
        let (store, _dir) = store();
        let id = with_attachment(&store, "../../escape.pdf", b"%PDF-1.7 pretend");
        let out = tempfile::tempdir().unwrap();

        let path = attach::save(&store, id, 0, out.path()).unwrap();

        assert_eq!(path, out.path().join("escape.pdf"));
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7 pretend");
        assert!(
            path.starts_with(out.path()),
            "it escaped the directory: {}",
            path.display()
        );
    }

    #[test]
    fn saving_twice_keeps_both_rather_than_replacing_one() {
        // Two messages with the same attachment name is ordinary — "invoice.pdf" from the same
        // supplier every month — and losing last month's to this month's is not acceptable.
        let (store, _dir) = store();
        let first = with_attachment(&store, "invoice.pdf", b"january");
        let second = with_attachment(&store, "invoice.pdf", b"february");
        let out = tempfile::tempdir().unwrap();

        let a = attach::save(&store, first, 0, out.path()).unwrap();
        let b = attach::save(&store, second, 0, out.path()).unwrap();

        assert_ne!(a, b, "the second overwrote the first");
        assert_eq!(std::fs::read(&a).unwrap(), b"january");
        assert_eq!(std::fs::read(&b).unwrap(), b"february");
        assert_eq!(b.file_name().unwrap(), "invoice (2).pdf");
    }

    #[test]
    fn asking_for_an_attachment_that_is_not_there_says_how_many_are() {
        let (store, _dir) = store();
        let id = with_attachment(&store, "one.pdf", b"x");
        let out = tempfile::tempdir().unwrap();

        let why = attach::save(&store, id, 7, out.path()).unwrap_err();
        assert!(why.contains("1 attachment"), "{why}");
        assert!(why.contains("no number 7"), "{why}");
    }

    #[test]
    fn the_listing_shows_the_name_that_will_be_written() {
        // Not the claim. A listing that showed `../../escape.pdf` would be describing something
        // that does not happen.
        let (store, _dir) = store();
        let id = with_attachment(&store, "../../escape.pdf", b"x");
        let out = attach::list(&store, id).unwrap();
        assert!(out.contains("escape.pdf"), "{out}");
        assert!(!out.contains(".."), "{out}");
        assert!(out.contains("application/pdf"), "{out}");
        assert!(out.contains("mailo save"), "it says how to get one: {out}");
    }

    #[test]
    fn a_message_with_nothing_attached_says_so() {
        // The shape of most mail. The first version of this test cleared the attachments on a
        // *copy* of the message and asserted a disjunction that was true either way — the
        // mistake CONVENTIONS §"An assertion that was already true proves nothing" is about,
        // made while writing the rule's own neighbours.
        let (store, _dir) = store();
        let id = plain_message(&store);
        let out = attach::list(&store, id).unwrap();
        assert!(out.contains("no attachments"), "{out}");
        assert!(!out.contains("mailo save"), "nothing to save: {out}");
    }
}

mod sizes {
    use super::*;

    #[test]
    fn sizes_are_readable_at_every_scale() {
        assert_eq!(attach::human_size(0), "0 B");
        assert_eq!(attach::human_size(512), "512 B");
        assert_eq!(attach::human_size(1024), "1.0 kB");
        assert_eq!(attach::human_size(1536), "1.5 kB");
        assert_eq!(attach::human_size(1024 * 1024), "1.0 MB");
        assert_eq!(attach::human_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }
}

/// The whole chain, from bytes a server would send to a file on disk.
///
/// Everything above builds an `Attachment` by hand. This one starts from RFC 5322 bytes with a
/// hostile `filename` parameter, ingests them the way a sync does, and saves what comes out —
/// so the MIME parser, the blob the runtime writes, the listing and the name are all the real
/// ones rather than four things that agree with each other in a fixture.
mod from_the_wire {
    use super::*;

    const RAW: &[u8] = b"From: Accounts <billing@example.test>\r\n\
To: ada@example.test\r\n\
Subject: your invoice\r\n\
Date: Tue, 22 Sep 2026 09:00:00 +0800\r\n\
Message-ID: <invoice@example.test>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: 7bit\r\n\
\r\n\
Invoice attached.\r\n\
--b1\r\n\
Content-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=\"../../escape/invoice.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQKMSAwIG9iajw8L1R5cGUvQ2F0YWxvZz4+ZW5kb2JqCg==\r\n\
--b1--\r\n";

    #[test]
    fn a_multipart_message_becomes_a_file_on_disk_under_a_name_it_did_not_choose() {
        let (store, _dir) = store();
        mail_runtime::absorb(
            &store,
            acct_account(),
            MailboxRef {
                account: acct_account(),
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![mail_runtime::Arrival {
                remote: RemoteRef::Pop {
                    uidl: "u1".to_owned(),
                },
                raw: RAW.to_vec(),
            }],
            false,
            chrono::Utc::now(),
        )
        .unwrap();

        let threads = store
            .threads(
                &Query {
                    filter: Filter::All,
                    sort: Sort {
                        property: Property::Date,
                        dir: SortDir::Desc,
                    },
                    page: PageReq {
                        after: None,
                        limit: 10,
                    },
                },
                chrono::Utc::now(),
            )
            .unwrap();
        let message_id = store.thread(threads.items[0].id).unwrap().messages[0];

        // The parser found it, the runtime stored its bytes, and the listing names what will be
        // written rather than what the sender asked for.
        let listed = attach::list(&store, message_id).unwrap();
        assert!(listed.contains("invoice.pdf"), "{listed}");
        assert!(
            !listed.contains(".."),
            "the claim reached the listing: {listed}"
        );
        assert!(listed.contains("application/pdf"), "{listed}");

        let out = tempfile::tempdir().unwrap();
        let path = attach::save(&store, message_id, 0, out.path()).unwrap();
        assert_eq!(path, out.path().join("invoice.pdf"));
        assert!(
            std::fs::read(&path).unwrap().starts_with(b"%PDF-1.4"),
            "the base64 was not decoded on the way through"
        );
        assert!(
            !out.path().join("..").join("..").join("escape").exists(),
            "something was written where the message asked"
        );
    }
}

/// Where the window puts a file when the reader is asked to save one — `plan.md` phase 7c.
///
/// The pane listed what was attached and then printed a command for the user to run, which was
/// honest while there was no way to save from the window and furniture once there was.
mod where_it_goes {
    use super::*;

    /// Absolute as this platform counts it: Windows wants a drive for that.
    const ELSEWHERE: &str = if cfg!(windows) {
        r"C:\somewhere-else"
    } else {
        "/tmp/somewhere-else"
    };

    /// `(row, the desktop's setting, home, where it goes)`.
    const CASES: &[(&str, Option<&str>, Option<&str>, &str)] = &[
        (
            "the desktop's own download directory wins",
            Some(ELSEWHERE),
            Some("/home/nobody"),
            ELSEWHERE,
        ),
        // A relative setting would put the file wherever the process was started, which for a
        // desktop launcher is somewhere the user cannot guess and cannot be told afterwards.
        (
            "a relative setting is ignored rather than followed",
            Some("Downloads"),
            Some("/home/nobody"),
            "/home/nobody/Downloads",
        ),
        (
            "without a setting it is Downloads under home",
            None,
            Some("/home/nobody"),
            "/home/nobody/Downloads",
        ),
        (
            "with no home at all it is here rather than nowhere",
            None,
            None,
            ".",
        ),
    ];

    #[test]
    fn the_download_directory_cases() {
        for &(row, setting, home, want) in CASES {
            assert_eq!(
                attach::downloads_from(
                    setting.map(std::ffi::OsStr::new),
                    home.map(std::ffi::OsStr::new)
                ),
                std::path::PathBuf::from(want),
                "{row}"
            );
        }
    }

    #[test]
    fn saving_twice_keeps_both_rather_than_overwriting() {
        // The reader's Save button is one click and can be clicked again. `free_path` already
        // refuses to overwrite; this is the property that button depends on. Into a directory
        // not made yet, which saving makes.
        let (store, dir) = store();
        let message = with_attachment(&store, "report.pdf", b"first");
        let into = dir.path().join("saved");

        let one = attach::save(&store, message, 0, &into).unwrap();
        let two = attach::save(&store, message, 0, &into).unwrap();
        assert_ne!(one, two, "the second save replaced the first");
        assert!(one.exists() && two.exists());
    }
}

/// An attachment a sync left on the server: downloaded when saved, and only then.
mod left_on_the_server {
    use super::*;

    /// The fixture's message, with its one attachment turned into a part still on the server.
    fn with_remote_part(store: &SqliteStore) -> MessageId {
        let id = with_attachment(store, "report.pdf", b"unused");
        mail_store::testing::edit_messages(store, |message| {
            if message.id != id {
                return false;
            }
            message.attachments = vec![Attachment {
                name: "report.pdf".to_owned(),
                mime: "application/pdf".to_owned(),
                size: 900,
                content: PartContent::Remote {
                    section: "2".to_owned(),
                },
                inline: Inline::Attached,
            }];
            true
        });
        id
    }

    #[test]
    fn saving_downloads_the_part_first_and_then_writes_it() {
        let (store, _dir) = store();
        let id = with_remote_part(&store);
        let out = tempfile::tempdir().unwrap();

        let mut asked = Vec::new();
        let said = attach::fetch_and_save(&store, id, 0, out.path(), |section| {
            asked.push(section.to_owned());
            // What the network half does: store the bytes, record the part as held.
            let blob = store.blobs().put(b"%PDF-1.4").map_err(|e| e.to_string())?;
            store
                .hold_part(id, section, blob, 8)
                .map_err(|e| e.to_string())
        })
        .unwrap();

        assert_eq!(asked, ["2"]);
        let path = out.path().join("report.pdf");
        assert_eq!(said, format!("Saved to {}", path.display()));
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.4");
    }

    #[test]
    fn a_part_already_here_is_not_downloaded_again() {
        let (store, _dir) = store();
        let id = with_attachment(&store, "report.pdf", b"held");
        let out = tempfile::tempdir().unwrap();
        attach::fetch_and_save(&store, id, 0, out.path(), |_| {
            panic!("a held part must not touch the network")
        })
        .unwrap();
        assert_eq!(
            std::fs::read(out.path().join("report.pdf")).unwrap(),
            b"held"
        );
    }

    #[test]
    fn a_failed_download_writes_nothing_and_says_why() {
        let (store, _dir) = store();
        let id = with_remote_part(&store);
        let out = tempfile::tempdir().unwrap();
        let err = attach::fetch_and_save(&store, id, 0, out.path(), |_| {
            Err("cannot download the attachment: connection refused".to_owned())
        })
        .unwrap_err();
        assert!(err.contains("connection refused"), "{err}");
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
    }

    /// `save` alone never reaches for the network, so a remote part is a clear refusal from it.
    #[test]
    fn plain_save_says_the_part_is_still_on_the_server() {
        let (store, _dir) = store();
        let id = with_remote_part(&store);
        let out = tempfile::tempdir().unwrap();
        let err = attach::save(&store, id, 0, out.path()).unwrap_err();
        assert!(err.contains("still on the server"), "{err}");
    }
}
