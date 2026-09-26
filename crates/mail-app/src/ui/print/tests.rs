#[cfg(feature = "webview")]
use super::window::{Heard, Move, Phase, decide};
use super::{Job, PrintTool, SAVED_AS, build, job_for, save_into, started};
use crate::ui::app::App;
use crate::ui::fixtures::{
    ACCOUNT, INSIDE_THE_SHELL, Scripts, chord, click, dispatching, rebuild_into, seeded, work,
};
use chrono::TimeZone;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use mail_domain::*;
use mail_mime::Pages;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
#[cfg(feature = "webview")]
use std::time::Duration;

const SUBJECT: &str = "Quarterly figures, and what they mean";

/// The line every printout carries, which no edit on the way may drop.
const CSP: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
                   img-src data:; style-src 'unsafe-inline'\">";

/// A conversation of two messages, the second with a script in it that must not survive.
fn conversation() -> (Arc<SqliteStore>, ThreadId, tempfile::TempDir) {
    let (store, dir) = seeded();
    let thread = ThreadId::generate();
    let bodies = [
        "Content-Type: text/plain; charset=utf-8\r\n\r\nThe figures are attached.\r\n",
        "Content-Type: text/html; charset=utf-8\r\n\r\n<p>Thanks.</p><script>alert(1)</script>\r\n",
    ];
    let fetched = bodies
        .iter()
        .enumerate()
        .map(|(index, body)| {
            let raw = format!(
                "From: Ada <ada@example.test>\r\nSubject: {SUBJECT}\r\nMIME-Version: 1.0\r\n{body}"
            );
            let raw = store
                .blobs()
                .put(&store.connection(), raw.as_bytes())
                .unwrap();
            let message = Message {
                id: MessageId::generate(),
                thread,
                account: ACCOUNT,
                key: MessageKey::Rfc(format!("print{index}@example.test")),
                date: chrono::Utc
                    .with_ymd_and_hms(2026, 9, 1 + index as u32, 9, 0, 0)
                    .unwrap(),
                from: Address {
                    name: Some("Ada".to_owned()),
                    email: "ada@example.test".to_owned(),
                },
                reply_to: vec![],
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: SUBJECT.to_owned(),
                in_reply_to: None,
                references: vec![],
                rfc_message_id: Some(format!("print{index}@example.test")),
                read: ReadState::Read,
                star: Star::Unstarred,
                mailbox: MailboxRole::Inbox,
                labels: vec![],
                body: Body::Present { text: None, raw },
                attachments: vec![],
            };
            Fetched {
                remote: RemoteRef::Pop {
                    uidl: format!("p{index}"),
                },
                key: message.key.clone(),
                raw,
                message,
            }
        })
        .collect();
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
                messages: fetched,
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    (store, thread, dir)
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc.with_ymd_and_hms(2026, 9, 24, 12, 0, 0).unwrap()
}

#[test]
fn the_two_page_choices_print_differently_and_both_are_safe() {
    let (store, thread, _dir) = conversation();
    let flow = build(
        &store,
        Job {
            thread,
            pages: Pages::Flow,
        },
        &chrono::Utc,
        now(),
    )
    .unwrap();
    let apart = build(
        &store,
        Job {
            thread,
            pages: Pages::PerMessage,
        },
        &chrono::Utc,
        now(),
    )
    .unwrap();
    assert_ne!(flow.html, apart.html, "the page choice changed nothing");
    for (name, printed) in [("flow", &flow), ("per message", &apart)] {
        assert_eq!(printed.subject, SUBJECT, "{name}");
        assert!(printed.html.contains(SUBJECT), "{name}: no subject");
        assert!(printed.html.contains("The figures are attached."), "{name}");
        assert!(printed.html.contains("Thanks."), "{name}");
        assert!(
            !printed.html.to_ascii_lowercase().contains("<script"),
            "{name}: a script reached the printout:\n{}",
            printed.html
        );
        assert!(printed.html.contains(CSP), "{name}: the CSP is gone");
    }
}

#[test]
fn save_for_printing_writes_beside_what_is_there_and_says_where() {
    let (store, thread, _dir) = conversation();
    let into = tempfile::tempdir().unwrap();
    let job = Job {
        thread,
        pages: Pages::Flow,
    };
    let first = save_into(&store, job, into.path(), &chrono::Utc, now());
    let second = save_into(&store, job, into.path(), &chrono::Utc, now());
    let one = into.path().join(format!("{SUBJECT}.{SAVED_AS}"));
    let two = into.path().join(format!("{SUBJECT} (2).{SAVED_AS}"));
    assert_eq!(first, format!("Saved for printing to {}", one.display()));
    assert_eq!(second, format!("Saved for printing to {}", two.display()));
    #[cfg(feature = "webview")]
    {
        let written = std::fs::read_to_string(&one).unwrap();
        assert!(written.contains(CSP) && written.contains(SUBJECT));
        assert_eq!(written, std::fs::read_to_string(&two).unwrap());
    }
    // The PDF's own text is `pdf_tests.rs`'s; here, only that both are one. Two PDFs of the same
    // document are not byte for byte the same (each file's `/ID` is its own), so no more.
    #[cfg(feature = "native")]
    for path in [&one, &two] {
        let bytes = std::fs::read(path).unwrap();
        assert!(
            bytes.starts_with(b"%PDF-"),
            "{} is not a PDF",
            path.display()
        );
    }
}

#[test]
fn a_thread_that_is_gone_says_so_rather_than_saving_nothing() {
    let (store, _dir) = seeded();
    let into = tempfile::tempdir().unwrap();
    let job = Job {
        thread: ThreadId::generate(),
        pages: Pages::Flow,
    };
    let said = save_into(&store, job, into.path(), &chrono::Utc, now());
    assert!(said.starts_with("Could not save for printing:"), "{said}");
    assert_eq!(std::fs::read_dir(into.path()).unwrap().count(), 0);
}

#[test]
fn ctrl_p_prints_the_open_thread_as_one_flow_and_nothing_else() {
    let thread = ThreadId::generate();
    assert_eq!(job_for(None), None);
    assert_eq!(
        job_for(Some(thread)),
        Some(Job {
            thread,
            pages: Pages::Flow
        })
    );
}

#[tokio::test]
async fn ctrl_p_with_nothing_open_does_nothing_and_with_a_thread_starts_printing() {
    dispatching();
    let built = work();
    let dana = built.dana;
    let scripts = Scripts::default();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store)
        .with_root_context(built.dirs)
        .with_root_context(scripts.document());
    let seen = rebuild_into(&mut dom);
    let shell = ElementId(INSIDE_THE_SHELL as usize);

    let before = dioxus_ssr::render(&dom);
    chord(&mut dom, "p", Modifiers::CONTROL, shell);
    assert!(started().is_empty(), "printed with nothing open");
    assert_eq!(
        dioxus_ssr::render(&dom),
        before,
        "Ctrl P with nothing open changed the window"
    );

    let row = seen.one(
        "aria-label",
        "Open Re: UIDL stability across a UIDVALIDITY change",
    );
    click(&mut dom, row);
    chord(&mut dom, "p", Modifiers::CONTROL, shell);
    assert_eq!(
        started(),
        vec![Job {
            thread: dana,
            pages: Pages::Flow
        }]
    );
}

#[component]
fn Head(thread: ThreadId) -> Element {
    rsx! { div { class: "bar-tools", PrintTool { thread } } }
}

#[tokio::test]
async fn the_menu_prints_the_pages_chosen_and_everything_it_draws_is_styled() {
    dispatching();
    let (store, thread, _dir) = conversation();
    let mut dom = VirtualDom::new_with_props(Head, HeadProps { thread }).with_root_context(store);
    let seen = rebuild_into(&mut dom);
    assert!(!dioxus_ssr::render(&dom).contains("print-menu"));

    let seen_open = click(&mut dom, seen.one("aria-label", "Print this conversation"));
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("print-menu"),
        "the menu did not open:\n{page}"
    );
    let missing =
        crate::ui::style::tests::unstyled_classes(&page, &crate::ui::style::tests::full_css());
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
    assert!(
        page.contains("aria-pressed=\"true\">Whole conversation<"),
        "the flow is not the default:\n{page}"
    );

    // The Pages segments, in order: the flow, then a page each.
    click(
        &mut dom,
        seen_open.after("aria-label", "Pages", "aria-pressed")[1],
    );
    click(&mut dom, seen_open.one("aria-label", "Print"));
    assert_eq!(
        started(),
        vec![Job {
            thread,
            pages: Pages::PerMessage
        }]
    );
    assert!(
        !dioxus_ssr::render(&dom).contains("print-menu"),
        "the menu stayed open after Print"
    );
}

#[test]
#[cfg(feature = "webview")]
fn a_print_window_closes_when_it_is_done_and_not_before() {
    let limit = Duration::from_secs(600);
    let heard = |loaded: bool, finished: bool, failed: Option<&str>| {
        let heard = Heard::default();
        heard.loaded.set(loaded);
        heard.finished.set(finished);
        heard.failed.replace(failed.map(str::to_owned));
        heard
    };
    let secs = Duration::from_secs;
    let cases: &[(&str, Phase, Heard, Duration, Move)] = &[
        (
            "still loading",
            Phase::Loading,
            heard(false, false, None),
            secs(1),
            Move::Wait,
        ),
        (
            "loaded",
            Phase::Loading,
            heard(true, false, None),
            secs(1),
            Move::Print,
        ),
        (
            "never loaded",
            Phase::Loading,
            heard(false, false, None),
            secs(31),
            Move::Close(Err(
                "The printout did not load, so nothing was printed.".to_owned()
            )),
        ),
        (
            "printing",
            Phase::Printing,
            heard(true, false, None),
            secs(5),
            Move::Wait,
        ),
        (
            "finished",
            Phase::Printing,
            heard(true, true, None),
            secs(5),
            Move::Close(Ok(())),
        ),
        (
            "past the limit",
            Phase::Printing,
            heard(true, false, None),
            secs(601),
            Move::Close(Ok(())),
        ),
        (
            "failed",
            Phase::Printing,
            heard(true, true, Some("no printer")),
            secs(5),
            Move::Close(Err("Printing failed: no printer".to_owned())),
        ),
    ];
    let mut failures = Vec::new();
    for (name, phase, heard, elapsed, expect) in cases {
        let got = decide(*phase, heard, *elapsed, limit);
        if got != *expect {
            failures.push(format!("{name}: got {got:?}, want {expect:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The reader head with Print's menu open, and a printed page, for screenshots.
///
/// ```text
/// cargo test -p mail-app -- --ignored render_printing_to_files
/// ```
#[tokio::test]
#[ignore = "writes target/print-menu.html and target/printed.html; run with --ignored"]
async fn render_printing_to_files() {
    dispatching();
    let (store, thread, _dir) = conversation();
    let printed = build(
        &store,
        Job {
            thread,
            pages: Pages::PerMessage,
        },
        &chrono::Utc,
        now(),
    )
    .unwrap();
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::write(target.join("printed.html"), &printed.html).unwrap();

    #[component]
    fn Opened(thread: ThreadId) -> Element {
        let shell = use_signal(|| crate::view::Shell {
            open: Some(thread),
            ..crate::view::Shell::default()
        });
        rsx! { crate::ui::reading::Reader { thread, shell } }
    }
    let mut dom =
        VirtualDom::new_with_props(Opened, OpenedProps { thread }).with_root_context(store);
    let seen = rebuild_into(&mut dom);
    click(&mut dom, seen.one("aria-label", "Print this conversation"));
    let body = dioxus_ssr::render(&dom);
    crate::ui::fixtures::dump(
        "print-menu",
        &format!(
            "<div class=\"reader\" style=\"width: 760px; height: 560px; margin: 0 auto\">{body}</div>"
        ),
    );
}
