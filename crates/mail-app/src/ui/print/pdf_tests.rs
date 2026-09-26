//! The printout as a PDF (`native`): made from a seeded conversation through the path Print and
//! Save for printing take, and read back with pdfrum as a reader of the PDF would, for its pages,
//! its text and its images. No test here opens a print dialog: the [`Printer`] is a recorder.

use super::native_print::{BUSY, Printer, print_through, said};
use super::paper::{self, Cjk, PICTURES_NOTE, Paper};
use super::{Job, build};
use crate::ui::fixtures::{ACCOUNT, seeded};
use chrono::TimeZone;
use ds_native::{PageSize, PrintError, PrintOutcome};
use mail_domain::*;
use mail_mime::Pages;
use mail_store::{SqliteStore, Store};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SUBJECT: &str = "Quarterly figures 季度報告";
/// A Latin word only the first message has.
const LATIN_WORD: &str = "kestrel";
/// A CJK sentence only the first message has.
const CJK_WORDS: &str = "會議紀錄已經寄出";
/// Only the second message has these.
const SECOND: &str = "Thanks, the chart is clear";
const SECOND_CJK: &str = "謝謝";
/// Only the third message has this.
const THIRD: &str = "One more thing about the budget";
/// The remote image's alt text and host, which the printout names instead of drawing.
const REMOTE_ALT: &str = "Team photo";
const REMOTE_HOST: &str = "images.example.test";

/// One message to seed: who sent it, to whom, its subject, its MIME body (headers from
/// `Content-Type` on), and the attachments it lists.
struct Seed {
    from: &'static str,
    to: Vec<String>,
    subject: String,
    mime: String,
    attached: Vec<(String, u64)>,
}

/// `seeds` stored as one conversation, a day apart and in order. Returns its thread.
fn thread_of(store: &SqliteStore, seeds: Vec<Seed>) -> ThreadId {
    let thread = ThreadId::generate();
    let tag = thread.as_uuid().simple().to_string();
    let fetched = seeds
        .into_iter()
        .enumerate()
        .map(|(index, seed)| {
            let raw = format!(
                "From: {}\r\nSubject: {}\r\nMIME-Version: 1.0\r\n{}",
                seed.from, seed.subject, seed.mime
            );
            let raw = store
                .blobs()
                .put(&store.connection(), raw.as_bytes())
                .unwrap();
            let key = format!("paper{index}.{tag}@example.test");
            let message = Message {
                id: MessageId::generate(),
                thread,
                account: ACCOUNT,
                key: MessageKey::Rfc(key.clone()),
                date: chrono::Utc
                    .with_ymd_and_hms(2026, 9, 1 + index as u32, 9, 0, 0)
                    .unwrap(),
                from: Address {
                    name: None,
                    email: seed.from.to_owned(),
                },
                reply_to: vec![],
                to: seed
                    .to
                    .into_iter()
                    .map(|email| Address { name: None, email })
                    .collect(),
                cc: vec![],
                bcc: vec![],
                subject: seed.subject,
                in_reply_to: None,
                references: vec![],
                rfc_message_id: Some(key.clone()),
                read: ReadState::Read,
                star: Star::Unstarred,
                mailbox: MailboxRole::Inbox,
                labels: vec![],
                body: Body::Present { text: None, raw },
                attachments: seed
                    .attached
                    .into_iter()
                    .enumerate()
                    .map(|(part, (name, size))| Attachment {
                        name,
                        mime: "application/octet-stream".to_owned(),
                        size,
                        content: PartContent::Remote {
                            section: (part + 2).to_string(),
                        },
                        inline: Inline::Attached,
                    })
                    .collect(),
            };
            Fetched {
                remote: RemoteRef::Pop {
                    uidl: format!("p{index}.{tag}"),
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
    thread
}

/// A 240 x 90 opaque PNG: a picture a message carries inline.
fn png() -> Vec<u8> {
    let chart = image::RgbImage::from_fn(240, 90, |x, y| {
        image::Rgb([(x * 255 / 240) as u8, 90, (y * 255 / 90) as u8])
    });
    let mut bytes = Vec::new();
    chart
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    let wide = base64::engine::general_purpose::STANDARD.encode(bytes);
    // MIME's 76-column lines.
    wide.as_bytes()
        .chunks(76)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Three messages: HTML in Latin and CJK with an inline (`cid:`) PNG, a remote image and two
/// attachments listed; then two plain replies, one with CJK.
fn conversation(store: &SqliteStore) -> ThreadId {
    let html = format!(
        "<p>The {LATIN_WORD} report is ready. {CJK_WORDS}。</p>\
         <p><img src=\"cid:chart@example.test\" alt=\"Chart\" width=\"240\" height=\"90\"></p>\
         <p><img src=\"https://{REMOTE_HOST}/photo.png\" alt=\"{REMOTE_ALT}\" width=\"200\" \
         height=\"100\"></p>"
    );
    let first = format!(
        "Content-Type: multipart/related; boundary=\"b1\"\r\n\r\n\
         --b1\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\
         {html}\r\n\
         --b1\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\n\
         Content-ID: <chart@example.test>\r\nContent-Disposition: inline; filename=\"chart.png\"\r\n\r\n\
         {}\r\n--b1--\r\n",
        base64(&png())
    );
    let plain = |text: &str| {
        format!(
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\
             {text}\r\n"
        )
    };
    thread_of(
        store,
        vec![
            Seed {
                from: "ada@example.test",
                to: vec!["grace@example.test".to_owned()],
                subject: SUBJECT.to_owned(),
                mime: first,
                attached: vec![
                    ("figures.xlsx".to_owned(), 20_480),
                    ("報告.pdf".to_owned(), 102_400),
                ],
            },
            Seed {
                from: "grace@example.test",
                to: vec!["ada@example.test".to_owned()],
                subject: format!("Re: {SUBJECT}"),
                mime: plain(&format!("{SECOND}. {SECOND_CJK}!")),
                attached: vec![],
            },
            Seed {
                from: "ada@example.test",
                to: vec!["grace@example.test".to_owned()],
                subject: format!("Re: {SUBJECT}"),
                mime: plain(&format!("{THIRD}.")),
                attached: vec![],
            },
        ],
    )
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc.with_ymd_and_hms(2026, 9, 24, 12, 0, 0).unwrap()
}

/// `job` as the PDF Print makes, on A4 with Traditional Chinese first.
fn printout(store: &SqliteStore, job: Job) -> Vec<u8> {
    let printed = build(store, job, &chrono::Utc, now()).unwrap();
    paper::pdf(&printed, &Paper::plain()).unwrap()
}

fn open(bytes: &[u8]) -> pdfrum::Document {
    pdfrum::Document::from_bytes(bytes.to_vec()).expect("the printout opens as a PDF")
}

/// The text of page `index` without whitespace, so a line the layout broke is still found.
fn squeezed(doc: &pdfrum::Document, index: u32) -> String {
    doc.page(index)
        .expect("the page exists")
        .text()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn all_text(doc: &pdfrum::Document) -> String {
    (0..doc.page_count())
        .map(|page| squeezed(doc, page))
        .collect()
}

fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn page_per_message_starts_each_message_on_a_page_of_its_own() {
    let (store, _dir) = seeded();
    let thread = conversation(&store);
    let flow = open(&printout(
        &store,
        Job {
            thread,
            pages: Pages::Flow,
        },
    ));
    assert_eq!(
        flow.page_count(),
        1,
        "a short thread in one flow is one page"
    );

    let apart = open(&printout(
        &store,
        Job {
            thread,
            pages: Pages::PerMessage,
        },
    ));
    assert_eq!(apart.page_count(), 3, "one page per message");
    let words = [LATIN_WORD, SECOND, THIRD].map(squeeze);
    for (page, own) in words.iter().enumerate() {
        let text = squeezed(&apart, page as u32);
        for (other, word) in words.iter().enumerate() {
            assert_eq!(
                text.contains(word.as_str()),
                other == page,
                "page {page} and {word:?}: {text}"
            );
        }
        assert!(text.contains(own.as_str()));
    }
    for page in apart.pages() {
        assert!((page.width() - 595.28).abs() < 0.1, "not A4 across");
        assert!((page.height() - 841.89).abs() < 0.1, "not A4 down");
    }
}

#[test]
fn the_latin_and_cjk_text_of_the_thread_can_be_copied_out() {
    let (store, _dir) = seeded();
    let thread = conversation(&store);
    let bytes = printout(
        &store,
        Job {
            thread,
            pages: Pages::Flow,
        },
    );
    let doc = open(&bytes);
    let text = all_text(&doc);
    for word in [
        SUBJECT,
        LATIN_WORD,
        CJK_WORDS,
        SECOND,
        SECOND_CJK,
        THIRD,
        "figures.xlsx",
        "報告.pdf",
        "ada@example.test",
    ] {
        assert!(
            text.contains(&squeeze(word)),
            "{word:?} is not in the text: {text}"
        );
    }
    // The CJK is set in the faces the printout names, not whatever the fallback finds: the serif
    // body's in Noto Serif CJK (fontique's own fallback is a sans), the headers' in Noto Sans
    // CJK. Needs the Noto CJK faces installed, as quire's own PDF tests do. (A variable CJK face
    // is named after its default instance, `...-Thin`, while it prints at the layout's weight.)
    let fonts: Vec<String> = doc
        .embedded_fonts()
        .into_iter()
        .map(|font| font.name)
        .collect();
    for face in ["NotoSerifCJK", "NotoSansCJK"] {
        assert!(
            fonts.iter().any(|name| name.contains(face)),
            "{face} was not embedded: {fonts:?}"
        );
    }
    assert!(
        !fonts.iter().any(|name| name.contains("DroidSansFallback")),
        "the CJK fell back: {fonts:?}"
    );
}

#[test]
fn the_inline_image_is_embedded_and_the_remote_one_is_named_not_drawn() {
    let (store, _dir) = seeded();
    let thread = conversation(&store);
    let bytes = printout(
        &store,
        Job {
            thread,
            pages: Pages::Flow,
        },
    );
    let doc = open(&bytes);
    let images: Vec<_> = doc.pages().flat_map(|page| page.images()).collect();
    assert_eq!(
        images
            .iter()
            .map(|image| (image.width, image.height))
            .collect::<Vec<_>>(),
        vec![(240, 90)],
        "the inline PNG, and nothing else, is drawn"
    );
    let text = all_text(&doc);
    assert!(
        text.contains(&squeeze(&format!(
            "[image: {REMOTE_ALT}, from {REMOTE_HOST}]"
        ))),
        "the remote image is not named: {text}"
    );
    assert!(
        text.contains(&squeeze(PICTURES_NOTE)),
        "the printout does not say pictures were left out: {text}"
    );
    let raw = String::from_utf8_lossy(&bytes);
    assert!(
        !raw.contains("photo.png"),
        "the remote image's address reached the file"
    );
}

/// A thread whose first message is `filler` paragraphs and four attachments, and whose second
/// has a tall header, so that one of the two blocks meets page 1's end for some `filler`.
fn tall(store: &SqliteStore, filler: usize) -> Job {
    let paragraphs: Vec<String> = (1..=filler)
        .map(|n| format!("Filler paragraph {n}, which takes one line."))
        .collect();
    let plain = |text: String| {
        format!(
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\
             {text}\r\n"
        )
    };
    let thread = thread_of(
        store,
        vec![
            Seed {
                from: "ada@example.test",
                to: vec!["grace@example.test".to_owned()],
                subject: "Budget".to_owned(),
                mime: plain(paragraphs.join("\r\n\r\n")),
                attached: [
                    "first-file.pdf",
                    "second-file.pdf",
                    "third-file.pdf",
                    "last-file.pdf",
                ]
                .into_iter()
                .map(|name| (name.to_owned(), 4_096))
                .collect(),
            },
            Seed {
                from: "bob@example.test",
                to: ["amy", "ben", "cat", "dan", "eve", "zed"]
                    .into_iter()
                    .map(|name| format!("{name}@example.test"))
                    .collect(),
                subject: "Budget minutes, part two".to_owned(),
                mime: plain("The second message.".to_owned()),
                attached: vec![],
            },
        ],
    );
    Job {
        thread,
        pages: Pages::Flow,
    }
}

/// The attachment list's text, and the second message's header's.
const BLOCKS: [(&str, &[&str]); 2] = [
    (
        "the attachment list",
        &["Attachments(4)", "first-file.pdf", "last-file.pdf"],
    ),
    (
        "the header",
        &[
            "Budgetminutes,parttwo",
            "bob@example.test",
            "zed@example.test",
        ],
    ),
];

/// Which of [`BLOCKS`] a page's end cuts in `doc`: a block whose text is on more than one page.
fn cut(doc: &pdfrum::Document) -> Vec<&'static str> {
    let pages: Vec<String> = (0..doc.page_count()).map(|p| squeezed(doc, p)).collect();
    BLOCKS
        .iter()
        .filter(|(_, words)| {
            let on: Vec<usize> = words
                .iter()
                .map(|word| {
                    pages
                        .iter()
                        .position(|page| page.contains(word))
                        .unwrap_or_else(|| panic!("{word} is not printed"))
                })
                .collect();
            on.iter().any(|page| *page != on[0])
        })
        .map(|(name, _)| *name)
        .collect()
}

#[test]
fn a_header_or_an_attachment_list_is_never_cut_by_a_page_end() {
    let (store, _dir) = seeded();
    // From well inside page 1 to past its end, two paragraphs (about 66 px) at a time: each
    // block is taller than a step, so for some count each one meets the page's end.
    let counts: Vec<usize> = (12..=30).step_by(2).collect();
    let jobs: Vec<(usize, String)> = counts
        .iter()
        .map(|&filler| {
            let printed = build(&store, tall(&store, filler), &chrono::Utc, now()).unwrap();
            (filler, paper::for_paper(&printed.html, Cjk::Tc))
        })
        .collect();
    // The same documents without the keep-together markers, to show each count is a real test.
    let unmarked = |html: &str| html.replace(" data-break-inside=\"avoid\"", "");
    let results: Vec<(usize, Vec<&str>, Vec<&str>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(filler, html)| {
                scope.spawn(move || {
                    let spec = Paper::plain().spec;
                    let kept = ds_native::pdf(html, spec).unwrap();
                    let loose = ds_native::pdf(&unmarked(html), spec).unwrap();
                    (*filler, cut(&open(&kept)), cut(&open(&loose)))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut unkept = Vec::new();
    for (filler, kept, loose) in &results {
        assert!(
            kept.is_empty(),
            "{filler} paragraphs: a page's end cut {kept:?}"
        );
        unkept.extend(loose.iter().copied());
    }
    for (name, _) in BLOCKS {
        assert!(
            unkept.contains(&name),
            "no count put {name} across a page's end even unmarked, so nothing was tested"
        );
    }
}

#[test]
fn the_markers_are_added_only_where_the_builder_wrote_the_tags() {
    // A subject spelling out the tags arrives escaped, and is left alone.
    let (store, _dir) = seeded();
    let sly = "<article class=\"message new-page\"> <header class=\"headers\">";
    let thread = thread_of(
        &store,
        vec![
            Seed {
                from: "ada@example.test",
                to: vec![],
                subject: sly.to_owned(),
                mime: "Content-Type: text/plain\r\n\r\nOne.\r\n".to_owned(),
                attached: vec![("a.txt".to_owned(), 3)],
            },
            Seed {
                from: "ada@example.test",
                to: vec![],
                subject: sly.to_owned(),
                mime: "Content-Type: text/plain\r\n\r\nTwo.\r\n".to_owned(),
                attached: vec![],
            },
        ],
    );
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
    let html = paper::for_paper(&printed.html, Cjk::Tc);
    assert_eq!(html.matches("data-break-before=\"page\"").count(), 1);
    // Two headers and one attachment list.
    assert_eq!(html.matches("data-break-inside=\"avoid\"").count(), 3);
    assert!(html.contains("&lt;article class=&quot;message new-page&quot;&gt;"));
    // The builder's own lines are still there, the CSP first among them.
    assert!(html.contains("Content-Security-Policy"));
    assert!(!html.contains(PICTURES_NOTE), "no picture was left out");
}

#[test]
fn the_locale_chooses_the_paper_and_the_cjk_face() {
    type Vars = &'static [(&'static str, &'static str)];
    let cases: &[(Vars, PageSize, Cjk)] = &[
        (&[], PageSize::A4, Cjk::Tc),
        (&[("LANG", "C.UTF-8")], PageSize::A4, Cjk::Tc),
        (&[("LANG", "en_US.UTF-8")], PageSize::Letter, Cjk::Tc),
        (&[("LANG", "en_GB.UTF-8")], PageSize::A4, Cjk::Tc),
        (&[("LANG", "zh_TW.UTF-8")], PageSize::A4, Cjk::Tc),
        (&[("LANG", "zh_CN.UTF-8")], PageSize::A4, Cjk::Sc),
        (&[("LANG", "zh_HK.UTF-8")], PageSize::A4, Cjk::Hk),
        (&[("LANG", "ja_JP.UTF-8")], PageSize::A4, Cjk::Jp),
        (&[("LANG", "ko_KR.UTF-8")], PageSize::A4, Cjk::Kr),
        (&[("LANG", "fr_CA.UTF-8")], PageSize::Letter, Cjk::Tc),
        // The category's own variable over LANG, and LC_ALL over both.
        (
            &[("LANG", "zh_TW.UTF-8"), ("LC_PAPER", "en_US.UTF-8")],
            PageSize::Letter,
            Cjk::Tc,
        ),
        (
            &[("LANG", "en_US.UTF-8"), ("LC_ALL", "ja_JP.UTF-8")],
            PageSize::A4,
            Cjk::Jp,
        ),
        (
            &[("LC_CTYPE", "zh_CN.UTF-8"), ("LANG", "en_US@euro")],
            PageSize::Letter,
            Cjk::Sc,
        ),
    ];
    let mut failures = Vec::new();
    for (vars, size, cjk) in cases {
        let paper = Paper::from_locale(|name| {
            vars.iter()
                .find(|(var, _)| *var == name)
                .map(|(_, value)| (*value).to_owned())
        });
        if paper.spec.size != *size || paper.cjk != *cjk {
            failures.push(format!(
                "{vars:?}: got {:?} {:?}, want {size:?} {cjk:?}",
                paper.spec.size, paper.cjk
            ));
        }
        assert_eq!(paper.spec.margins, ds_native::Margins::default());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What reached a print dialog: each PDF's length, and its title.
type Seen = Arc<Mutex<Vec<(usize, String)>>>;

/// A printer that records what reached its dialog and answers `answer`.
fn recorder(answer: fn() -> Result<PrintOutcome, PrintError>) -> (Printer, Seen) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let printer = Printer::with_dialog({
        let seen = Arc::clone(&seen);
        move |pdf, title| {
            assert!(pdf.starts_with(b"%PDF-"), "the dialog was not given a PDF");
            seen.lock().unwrap().push((pdf.len(), title.to_owned()));
            answer()
        }
    });
    (printer, seen)
}

#[test]
fn print_hands_the_pdf_to_the_dialog_and_says_what_became_of_it() {
    let (store, _dir) = seeded();
    let thread = conversation(&store);
    let job = Job {
        thread,
        pages: Pages::Flow,
    };
    let expected_len = printout(&store, job).len();
    type Answer = fn() -> Result<PrintOutcome, PrintError>;
    let cases: [(Answer, String); 4] = [
        (
            || Ok(PrintOutcome::Printed),
            "Sent to the printer.".to_owned(),
        ),
        (
            || Ok(PrintOutcome::Cancelled),
            "Printing cancelled; nothing was printed.".to_owned(),
        ),
        (
            || Ok(PrintOutcome::Opened(PathBuf::from("/tmp/Quarterly-1.pdf"))),
            "There is no print dialog here, so the printout opened in your PDF viewer to \
             print from there: /tmp/Quarterly-1.pdf"
                .to_owned(),
        ),
        (
            || Err(PrintError::NoViewer("xdg-open is missing".to_owned())),
            "Could not print: no viewer opened the PDF: xdg-open is missing".to_owned(),
        ),
    ];
    for (answer, words) in cases {
        let (printer, seen) = recorder(answer);
        let got = print_through(&store, job, &Paper::plain(), &printer, &chrono::Utc, now());
        assert_eq!(got, words);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen, vec![(expected_len, SUBJECT.to_owned())]);
    }
}

#[test]
fn a_thread_that_is_gone_is_said_and_no_dialog_opens() {
    let (store, _dir) = seeded();
    let (printer, seen) = recorder(|| Ok(PrintOutcome::Printed));
    let got = print_through(
        &store,
        Job {
            thread: ThreadId::generate(),
            pages: Pages::Flow,
        },
        &Paper::plain(),
        &printer,
        &chrono::Utc,
        now(),
    );
    assert!(got.starts_with("Could not print:"), "{got}");
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn one_print_at_a_time() {
    let (printer, _) = recorder(|| Ok(PrintOutcome::Cancelled));
    let first = printer.claim().expect("a free printer");
    assert!(printer.claim().is_none(), "a second print started");
    drop(first);
    assert!(printer.claim().is_some(), "the printer stayed taken");
    // What the second Print says is a sentence, not a code.
    assert!(BUSY.ends_with('.'));
    assert_eq!(
        said(Ok(PrintOutcome::Cancelled)),
        "Printing cancelled; nothing was printed."
    );
}

/// A sample printout, for looking at: a CJK-and-English thread with an inline image.
///
/// ```text
/// MAILO_SAMPLE_PDF=/path/sample-print.pdf cargo test -p mail-app --no-default-features \
///     --features native -- --ignored write_a_sample_printout
/// ```
#[test]
#[ignore = "writes the file MAILO_SAMPLE_PDF names; run with --ignored"]
fn write_a_sample_printout() {
    let (store, _dir) = seeded();
    let thread = conversation(&store);
    let bytes = printout(
        &store,
        Job {
            thread,
            pages: Pages::PerMessage,
        },
    );
    let path = std::env::var("MAILO_SAMPLE_PDF").expect("MAILO_SAMPLE_PDF names the file");
    std::fs::write(&path, &bytes).unwrap();
    eprintln!(
        "{path}: {} pages, {} bytes",
        open(&bytes).page_count(),
        bytes.len()
    );
}
