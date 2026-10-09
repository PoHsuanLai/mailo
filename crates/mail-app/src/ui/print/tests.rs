use super::tool::{PrintChoice, ROWS, job_of};
use super::{Job, PrintTool, SAVED_AS, Sources, build, save_into, started};
use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Scripts, acct_account, chord, click, dispatching, rebuild_into, seeded, work,
};
use chrono::TimeZone;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use mail_domain::*;
use mail_mime::Pages;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

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
                .put(raw.as_bytes())
                .unwrap();
            let message = Message {
                id: MessageId::generate(),
                thread,
                account: acct_account(),
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
            acct_account(),
            Ingest {
                mailbox: MailboxRef {
                    account: acct_account(),
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
    let first = save_into(
        &store,
        job,
        &Sources::none(),
        into.path(),
        &chrono::Utc,
        now(),
    );
    let second = save_into(
        &store,
        job,
        &Sources::none(),
        into.path(),
        &chrono::Utc,
        now(),
    );
    let one = into.path().join(format!("{SUBJECT}.{SAVED_AS}"));
    let two = into.path().join(format!("{SUBJECT} (2).{SAVED_AS}"));
    assert_eq!(first, Ok(one.clone()));
    assert_eq!(second, Ok(two.clone()));
    // The PDF's own text is `pdf_tests.rs`'s; here, only that both are one. Two PDFs of the same
    // document are not byte for byte the same (each file's `/ID` is its own), so no more.
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
    let said = save_into(
        &store,
        job,
        &Sources::none(),
        into.path(),
        &chrono::Utc,
        now(),
    )
    .unwrap_err();
    assert!(said.starts_with("Could not save for printing:"), "{said}");
    assert_eq!(std::fs::read_dir(into.path()).unwrap().count(), 0);
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
        "⌘P with nothing open changed the window"
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
    rsx! { ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window, div { class: "bar-tools", PrintTool { thread } } } }
}

#[test]
fn each_row_of_the_menu_starts_its_own_job() {
    let thread = ThreadId::generate();
    let cases: &[(&str, PrintChoice, Pages)] = &[
        (
            "Print Conversation…",
            PrintChoice::Print(Pages::Flow),
            Pages::Flow,
        ),
        (
            "Print Each Message Separately…",
            PrintChoice::Print(Pages::PerMessage),
            Pages::PerMessage,
        ),
        ("Save Conversation as PDF…", PrintChoice::Save, Pages::Flow),
    ];
    let rows: Vec<(PrintChoice, &str)> = ROWS.into_iter().flatten().collect();
    assert_eq!(rows.len(), cases.len());
    for ((choice, title), (want_title, want_choice, pages)) in rows.into_iter().zip(cases) {
        assert_eq!(title, *want_title);
        assert_eq!(choice, *want_choice, "{title}");
        assert_eq!(
            job_of(thread, choice),
            Job {
                thread,
                pages: *pages
            },
            "{title}"
        );
    }
    // The rule sits between printing and saving.
    assert_eq!(ROWS[2], None);
}

#[tokio::test]
async fn print_opens_a_menu_whose_rows_print_the_pages_they_name() {
    dispatching();
    let (store, thread, _dir) = conversation();
    for (row, pages) in [
        ("Print Conversation…", Pages::Flow),
        ("Print Each Message Separately…", Pages::PerMessage),
    ] {
        let mut dom =
            VirtualDom::new_with_props(Head, HeadProps { thread }).with_root_context(store.clone());
        let seen = rebuild_into(&mut dom);
        assert!(!dioxus_ssr::render(&dom).contains("role=\"menu\""));
        let opened = click(&mut dom, seen.one("aria-label", "Print this conversation"))
            .merge(crate::ui::fixtures::drain_seen(&mut dom));
        let page = dioxus_ssr::render(&dom);
        let offences = crate::ui::style::tests::markup_offences(&page);
        assert!(offences.is_empty(), "the markup lint: {offences:#?}");
        // A standard pop-up menu: the three rows, a rule between printing and saving, and no
        // popover of controls.
        assert_eq!(
            crate::ui::fixtures::menu_names(&page),
            [
                "Print Conversation…",
                "Print Each Message Separately…",
                "Save Conversation as PDF…"
            ],
            "{page}"
        );
        assert!(page.contains("ds-menu-separator"), "{page}");
        assert!(!page.contains("print-menu") && !page.contains("class=\"ds-segmented"));
        let before = started().len();
        crate::ui::fixtures::pick_named(&mut dom, &opened, row).await;
        assert_eq!(started()[before..], [Job { thread, pages }], "{row}");
        crate::ui::fixtures::drain(&mut dom);
        assert!(
            !dioxus_ssr::render(&dom).contains("role=\"menu\""),
            "the menu stayed open after {row}"
        );
    }
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
        let shell = use_signal(|| crate::ui::view::Shell {
            open: Some(thread),
            ..crate::ui::view::Shell::default()
        });
        rsx! { ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window, crate::ui::reading::Reader { thread, shell } } }
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
