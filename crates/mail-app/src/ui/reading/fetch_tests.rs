use super::*;
use crate::ui::fixtures::{ACCOUNT, click, dispatching, held_and_remote, rebuild_into, seeded};
use crate::ui::view::Shell;
use dioxus_core::VirtualDom;
use mail_domain::*;
use std::time::Duration;

fn failed(retry: Retry, why: &str) -> BodyState {
    BodyState::Failed {
        retry,
        why: why.to_owned(),
    }
}

fn running() -> Operation {
    Operation::Running(PendingToken::start())
}

#[test]
fn a_missing_body_asks_to_be_downloaded() {
    assert_eq!(
        body_face(&BodyState::Missing, Operation::Idle, "a@x.test"),
        BodyFace::Prompt
    );
    assert_eq!(PROMPT, "This message hasn't been downloaded yet.");
    assert_eq!(DOWNLOAD_MESSAGE, "Download Message");
}

#[test]
fn a_fetching_body_is_loading_under_its_operation() {
    let op = running();
    assert_eq!(
        body_face(&BodyState::Fetching, op, "a@x.test"),
        BodyFace::Pane(Phase::Loading(op), Again::Withhold)
    );
}

#[test]
fn a_failed_body_says_what_to_do_about_it() {
    let cases = [
        (
            Retry::Now,
            "connection reset",
            "Check your connection and try again. (connection reset)",
            Again::Offer,
        ),
        (
            Retry::After(std::time::Duration::from_secs(5)),
            "",
            "Check your connection and try again.",
            Again::Offer,
        ),
        (
            Retry::NeedsReauth,
            "401",
            "Sign in to ada@example.test again to download it.",
            Again::Withhold,
        ),
        (
            Retry::Fatal("POP3 cannot fetch one message".into()),
            "POP3 cannot fetch one message",
            "This account's messages download with the next sync.",
            Again::Withhold,
        ),
    ];
    for (retry, why, said, again) in cases {
        let face = body_face(
            &failed(retry.clone(), why),
            Operation::Idle,
            "ada@example.test",
        );
        assert_eq!(
            face,
            BodyFace::Pane(
                Phase::Failed {
                    title: "Couldn't download this message".to_owned(),
                    description: Some(TextLine::from(said)),
                },
                again
            ),
            "{retry:?}"
        );
    }
}

#[test]
fn the_raw_reason_is_one_short_line() {
    assert_eq!(one_line("first\nsecond"), "first");
    assert_eq!(one_line("\n  \n  spaced  \nnext"), "spaced");
    let long = "x".repeat(300);
    let line = one_line(&long);
    assert_eq!(line.chars().count(), 121, "{line}");
    assert!(line.ends_with('…'));
}

#[test]
fn only_imap_and_graph_fetch_when_opened() {
    let imap = Incoming::Imap {
        host: "h".into(),
        port: 993,
        tls: Tls::Implicit,
    };
    assert!(opens_with_fetch(&imap));
    assert!(opens_with_fetch(&Incoming::Graph));
    assert!(!opens_with_fetch(&Incoming::Local));
}

#[test]
fn a_failed_download_names_the_file_and_keeps_the_raw_error_secondary() {
    let said = download_failure(
        "report.pdf",
        &Retry::Now,
        "tcp: broken pipe\nat x",
        "a@x.test",
    );
    assert_eq!(
        said,
        Failure {
            text: "Couldn't download report.pdf.".to_owned(),
            detail: "tcp: broken pipe".to_owned(),
            again: Again::Offer,
        }
    );
    let said = download_failure("a.pdf", &Retry::NeedsReauth, "401", "a@x.test");
    assert_eq!(said.detail, "Sign in to a@x.test again.");
    assert_eq!(said.again, Again::Withhold);
}

#[test]
fn a_saved_toast_names_the_folder() {
    assert_eq!(
        saved_toast(Path::new("/home/ada/Downloads/report.pdf")),
        "Saved to Downloads"
    );
    assert_eq!(saved_toast(Path::new("report.pdf")), "Saved");
}

// --- The reader -------------------------------------------------------------------------------

/// A thread whose only message has headers and no body, on the fixture's account (whose plan is
/// not one that fetches on open, so the prompt is what is drawn).
fn headers_only() -> (Arc<SqliteStore>, ThreadId, tempfile::TempDir) {
    let (store, dir) = seeded();
    let thread = ThreadId::generate();
    let message = Message {
        id: MessageId::generate(),
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc("headers@example.test".to_owned()),
        date: chrono::Utc::now(),
        from: Address {
            name: Some("Ada".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "only headers".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("headers@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: "headers".to_owned(),
                    },
                    key: message.key.clone(),
                    raw: BlobId::generate(),
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    (store, thread, dir)
}

#[component]
fn Open(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    rsx! { Ds { appearance: Appearance::default(), material: Material::Window, super::super::Reader { thread, shell } } }
}

fn failing() -> Fetchers {
    Fetchers {
        body: Arc::new(|_, _| Err((Retry::Now, "the server hung up".to_owned()))),
        part: Arc::new(|_, _, _| Err("the part is gone".to_owned())),
    }
}

async fn settle(dom: &mut VirtualDom, done: impl Fn(&str) -> bool) -> String {
    for _ in 0..40 {
        let markup = dioxus_ssr::render(dom);
        if done(&markup) {
            return markup;
        }
        tokio::time::timeout(Duration::from_millis(50), dom.wait_for_work())
            .await
            .ok();
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }
    dioxus_ssr::render(dom)
}

fn unescaped(markup: &str) -> String {
    markup.replace("&#39;", "'")
}

#[tokio::test]
async fn a_message_without_a_body_shows_the_download_button() {
    let (store, thread, _dir) = headers_only();
    let markup = unescaped(&crate::ui::fixtures::reader_markup(store, thread));
    assert!(
        markup.contains("This message hasn't been downloaded yet."),
        "{markup}"
    );
    assert!(
        markup.contains(">Download Message</span></button>"),
        "{markup}"
    );
    assert!(!markup.contains("Not downloaded"), "{markup}");
}

#[tokio::test]
async fn a_failed_fetch_shows_the_failure_with_retry() {
    dispatching();
    let (store, thread, _dir) = headers_only();
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread })
        .with_root_context(store)
        .with_root_context(failing());
    let seen = rebuild_into(&mut dom);
    let button = seen.one("aria-label", "Download Message");
    click(&mut dom, button);
    let markup = unescaped(
        &settle(&mut dom, |m| {
            m.contains("Couldn&#39;t download this message")
        })
        .await,
    );
    assert!(
        markup.contains("Couldn't download this message"),
        "{markup}"
    );
    assert!(
        markup.contains("Check your connection and try again. (the server hung up)"),
        "{markup}"
    );
    assert!(markup.contains(">Retry</span></button>"), "{markup}");
    assert!(!markup.contains(">Download Message</span>"), "{markup}");
}

#[tokio::test]
async fn an_attachment_that_will_not_download_says_so_in_a_danger_banner() {
    dispatching();
    let (store, _dir) = held_and_remote();
    let thread = crate::ui::fixtures::thread_like(&store, "quarterly");
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread })
        .with_root_context(store)
        .with_root_context(failing());
    let seen = rebuild_into(&mut dom);
    let button = seen.one("aria-label", "Download report.pdf");
    click(&mut dom, button);
    let markup = unescaped(
        &settle(&mut dom, |m| {
            m.contains("Couldn&#39;t download report.pdf.")
        })
        .await,
    );
    assert!(markup.contains("Couldn't download report.pdf."), "{markup}");
    assert!(markup.contains("the part is gone"), "{markup}");
    assert!(markup.contains("Try Again"), "{markup}");
    assert!(
        !markup.contains(r#"data-severity="ok""#),
        "a failure drew an Ok banner:\n{markup}"
    );
    assert!(markup.contains(r#"data-severity="danger""#), "{markup}");
}
