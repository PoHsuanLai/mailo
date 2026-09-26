//! The printout as a PDF (`native`): made from a seeded conversation through the path Print and
//! Save for printing take, and read back with pdfrum as a reader of the PDF would, for its pages,
//! its text and its images. No test here opens a print dialog: the [`Printer`] is a recorder.

use super::native_print::{BUSY, Printer, Sources, made, print_through, said};
use super::paper::{self, Cjk, Families, PICTURES_NOTE, Paper};
use super::{Job, build};
use crate::print::Printed;
use crate::ui::fixtures::{ACCOUNT, seeded};
use crate::ui::original::{Consent, FetchImage, Got, ReaderNet};
use chrono::TimeZone;
use ds_native::{PageSize, PrintError, PrintOutcome};
use mail_domain::*;
use mail_mime::{Pages, Script};
use mail_store::{SqliteStore, Store};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

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

/// `job` as the document Print makes, on A4 with Traditional Chinese first, fetching through
/// `sources`.
fn paper_document(store: &SqliteStore, job: Job, sources: &Sources) -> Printed {
    made(store, job, &Paper::plain(), sources, &chrono::Utc, now()).unwrap()
}

/// `job` as the PDF Print makes, on A4 with Traditional Chinese first, with no consent.
fn printout(store: &SqliteStore, job: Job) -> Vec<u8> {
    printout_with(store, job, &Sources::default())
}

/// [`printout`], with the remote images `sources` consents to.
fn printout_with(store: &SqliteStore, job: Job, sources: &Sources) -> Vec<u8> {
    paper::pdf(&paper_document(store, job, sources), &Paper::plain()).unwrap()
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
            let printed = paper_document(&store, tall(&store, filler), &Sources::default());
            (filler, printed.html)
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
fn the_markers_are_the_builders_and_a_subject_cannot_add_one() {
    // A subject spelling out the tags, markers and all, arrives escaped and marks nothing.
    let (store, _dir) = seeded();
    let sly = "<article class=\"message new-page\" data-break-before=\"page\"> \
               <header class=\"headers\" data-break-inside=\"avoid\">";
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
    let html = paper_document(
        &store,
        Job {
            thread,
            pages: Pages::PerMessage,
        },
        &Sources::default(),
    )
    .html;
    assert_eq!(html.matches("data-break-before=\"page\"").count(), 1);
    // Two headers and one attachment list.
    assert_eq!(html.matches("data-break-inside=\"avoid\"").count(), 3);
    assert!(html.contains("&lt;article class=&quot;message new-page&quot; data-break-before"));
    // The builder's own lines are there, the CSP first among them, and the faces after its
    // stylesheet.
    assert!(html.contains("Content-Security-Policy"));
    assert!(html.contains("'Noto Serif CJK TC'"));
    assert!(!html.contains(PICTURES_NOTE), "no picture was left out");
    // The webview's document is the same one, without the faces: the markers are the builder's.
    let plain = build(
        &store,
        Job {
            thread,
            pages: Pages::PerMessage,
        },
        &chrono::Utc,
        now(),
    )
    .unwrap()
    .html;
    assert_eq!(plain.matches("data-break-before=\"page\"").count(), 1);
    assert_eq!(plain.matches("data-break-inside=\"avoid\"").count(), 3);
    assert!(!plain.contains("Noto Serif CJK"));
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

#[test]
fn each_script_leads_with_its_own_regional_face() {
    let cases = [
        (Script::TraditionalChinese, Cjk::Tc, Cjk::Tc),
        (Script::TraditionalChinese, Cjk::Hk, Cjk::Hk),
        (Script::TraditionalChinese, Cjk::Sc, Cjk::Tc),
        (Script::SimplifiedChinese, Cjk::Tc, Cjk::Sc),
        (Script::Japanese, Cjk::Tc, Cjk::Jp),
        (Script::Korean, Cjk::Sc, Cjk::Kr),
    ];
    for (script, default, want) in cases {
        assert_eq!(Cjk::for_script(script, default), want, "{script:?}");
    }
    let stacks = [
        (
            Cjk::Tc,
            "'Georgia', 'Times New Roman', 'Noto Serif', 'Noto Serif CJK TC', \
             'Noto Serif CJK HK', 'Noto Serif CJK SC', 'Noto Serif CJK JP', \
             'Noto Serif CJK KR', serif",
        ),
        (
            Cjk::Sc,
            "'Georgia', 'Times New Roman', 'Noto Serif', 'Noto Serif CJK SC', \
             'Noto Serif CJK TC', 'Noto Serif CJK HK', 'Noto Serif CJK JP', \
             'Noto Serif CJK KR', serif",
        ),
        (
            Cjk::Jp,
            "'Georgia', 'Times New Roman', 'Noto Serif', 'Noto Serif CJK JP', \
             'Noto Serif CJK TC', 'Noto Serif CJK HK', 'Noto Serif CJK SC', \
             'Noto Serif CJK KR', serif",
        ),
        (
            Cjk::Kr,
            "'Georgia', 'Times New Roman', 'Noto Serif', 'Noto Serif CJK KR', \
             'Noto Serif CJK TC', 'Noto Serif CJK HK', 'Noto Serif CJK SC', \
             'Noto Serif CJK JP', serif",
        ),
    ];
    for (cjk, serif) in stacks {
        assert_eq!(Families::led_by(cjk).serif, serif, "{cjk:?}");
    }
    assert_eq!(
        Families::led_by(Cjk::Jp).sans,
        "'Noto Sans', 'Noto Sans CJK JP', 'Noto Sans CJK TC', 'Noto Sans CJK HK', \
         'Noto Sans CJK SC', 'Noto Sans CJK KR', sans-serif"
    );
    // The locale's face leads the document; a marked message's own script leads the message.
    let css = paper::paper_css(Cjk::Tc);
    assert!(css.starts_with(&format!(
        "body {{ font-family: {}; }}",
        Families::led_by(Cjk::Tc).serif
    )));
    for (tag, cjk) in [("zh-Hans", Cjk::Sc), ("ja", Cjk::Jp), ("ko", Cjk::Kr)] {
        let rule = format!(
            "article[data-script=\"{tag}\"] {{ font-family: {}; }}",
            Families::led_by(cjk).serif
        );
        assert!(css.contains(&rule), "no rule for {tag}: {css}");
    }
    // Traditional Chinese is what the document leads with already.
    assert!(!css.contains("data-script=\"zh-Hant\""), "{css}");
}

/// A one-message thread of `subject` and a plain `body`, as a PDF on A4 with Traditional Chinese
/// first: the names of the faces it embeds.
fn faces_for(subject: &str, body: &str) -> Vec<String> {
    let (store, _dir) = seeded();
    let thread = thread_of(
        &store,
        vec![Seed {
            from: "ada@example.test",
            to: vec!["grace@example.test".to_owned()],
            subject: subject.to_owned(),
            mime: format!(
                "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\
                 {body}\r\n"
            ),
            attached: vec![],
        }],
    );
    let bytes = printout(
        &store,
        Job {
            thread,
            pages: Pages::Flow,
        },
    );
    open(&bytes)
        .embedded_fonts()
        .into_iter()
        .map(|font| font.name)
        .collect()
}

#[test]
fn japanese_and_korean_mail_print_in_their_own_faces_on_a_chinese_locale() {
    // Needs the Noto CJK faces installed, as the test above does. A face in the collection is
    // named for its region: `NotoSerifCJKjp-...`, `NotoSansCJKkr-...`.
    let cases = [
        (
            "会議のお知らせ",
            "明日の会議は十時からです。よろしくお願いします。",
            "jp",
        ),
        (
            "회의 안내",
            "내일 회의는 열 시에 시작합니다. 감사합니다.",
            "kr",
        ),
    ];
    for (subject, body, region) in cases {
        let fonts = faces_for(subject, body);
        for face in ["NotoSerifCJK", "NotoSansCJK"] {
            let wanted = format!("{face}{region}");
            assert!(
                fonts.iter().any(|name| name.contains(&wanted)),
                "{wanted} was not embedded: {fonts:?}"
            );
        }
        assert!(
            !fonts.iter().any(|name| name.contains("CJKtc")),
            "the {region} message was set in the Traditional Chinese face: {fonts:?}"
        );
        assert!(
            !fonts.iter().any(|name| name.contains("DroidSansFallback")),
            "the CJK fell back: {fonts:?}"
        );
    }
}

/// A remote image's address, alt text and size, as [`with_a_remote_image`] has it.
const PHOTO: &str = "https://images.example.test/team.png";
const PHOTO_ALT: &str = "Team photo";

/// A 200 x 100 opaque PNG: what the photo's server sends.
fn photo() -> Vec<u8> {
    let photo = image::RgbImage::from_fn(200, 100, |x, _| image::Rgb([(x % 255) as u8, 40, 90]));
    let mut bytes = Vec::new();
    photo
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

/// A thread of one HTML message with the photo in it, and a tracking pixel.
fn with_a_remote_image(store: &SqliteStore) -> ThreadId {
    let html = format!(
        "<p>Our team.</p>\
         <p><img src=\"{PHOTO}\" alt=\"{PHOTO_ALT}\" width=\"200\" height=\"100\"></p>\
         <p><img src=\"https://track.example.test/open.gif\" width=\"1\" height=\"1\"></p>"
    );
    thread_of(
        store,
        vec![Seed {
            from: "ada@example.test",
            to: vec!["grace@example.test".to_owned()],
            subject: "The team".to_owned(),
            mime: format!(
                "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\
                 {html}\r\n"
            ),
            attached: vec![],
        }],
    )
}

/// A fetcher that records every URL asked for and answers each at once with [`photo`], after
/// running `before` (a test's chance to change the consent while the fetch is out).
#[derive(Clone, Default)]
struct Net {
    asked: Arc<Mutex<Vec<String>>>,
    before: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl FetchImage for Net {
    fn get(&self, url: String, done: Box<dyn FnOnce(Got) + Send>) {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url);
        if let Some(before) = &self.before {
            before();
        }
        done(Got {
            content_type: Some("image/png".to_owned()),
            bytes: photo(),
        });
    }
}

impl Net {
    fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The consent the reader writes when it shows `thread` with its images allowed.
fn consented(store: &SqliteStore, thread: ThreadId) -> Consent {
    let consent = Consent::new();
    let messages = store.thread(thread).unwrap().messages;
    consent.hold(
        consent.holder(),
        thread,
        Some(messages.into_iter().map(|id| (id, vec![])).collect()),
    );
    consent
}

/// The sizes of the images `bytes` draws.
fn drawn(bytes: &[u8]) -> Vec<(u32, u32)> {
    open(bytes)
        .pages()
        .flat_map(|page| page.images())
        .map(|image| (image.width, image.height))
        .collect()
}

fn one_flow(thread: ThreadId) -> Job {
    Job {
        thread,
        pages: Pages::Flow,
    }
}

#[test]
fn with_consent_the_remote_image_prints_from_what_the_readers_fetcher_brought() {
    let (store, _dir) = seeded();
    let thread = with_a_remote_image(&store);
    let net = Net::default();
    let sources = Sources::new(consented(&store, thread), ReaderNet(Arc::new(net.clone())));
    let bytes = printout_with(&store, one_flow(thread), &sources);
    // Asked for once, and the pixel not at all: it would print as nothing.
    assert_eq!(net.asked(), [PHOTO]);
    assert_eq!(drawn(&bytes), [(200, 100)], "the photo is drawn");
    let text = all_text(&open(&bytes));
    assert!(
        !text.contains(&squeeze(PHOTO_ALT)),
        "the photo is named: {text}"
    );
    assert!(!text.contains(&squeeze(PICTURES_NOTE)), "{text}");
    // The file holds the picture, not where it came from.
    assert!(!String::from_utf8_lossy(&bytes).contains("team.png"));
}

#[test]
fn without_consent_nothing_is_fetched_and_the_remote_image_is_named() {
    let (store, _dir) = seeded();
    let thread = with_a_remote_image(&store);
    let other = with_a_remote_image(&store);
    let cases = [
        ("no consent", Consent::new()),
        ("another thread's consent", consented(&store, other)),
    ];
    for (name, consent) in cases {
        let net = Net::default();
        let sources = Sources::new(consent, ReaderNet(Arc::new(net.clone())));
        let bytes = printout_with(&store, one_flow(thread), &sources);
        assert!(net.asked().is_empty(), "{name}: fetched {:?}", net.asked());
        assert!(drawn(&bytes).is_empty(), "{name}: an image was drawn");
        let text = all_text(&open(&bytes));
        assert!(
            text.contains(&squeeze(&format!(
                "[image: {PHOTO_ALT}, from images.example.test]"
            ))),
            "{name}: the photo is not named: {text}"
        );
        assert!(text.contains(&squeeze(PICTURES_NOTE)), "{name}: {text}");
    }
}

#[test]
fn consent_taken_back_while_the_image_is_coming_prints_it_named() {
    let (store, _dir) = seeded();
    let thread = with_a_remote_image(&store);
    let consent = consented(&store, thread);
    let net = Net {
        before: Some(Arc::new({
            let consent = consent.clone();
            // The reader closes, or opens another thread, while the fetch is out.
            move || consent.hold(consent.holder(), ThreadId::generate(), None)
        })),
        ..Net::default()
    };
    let sources = Sources::new(consent, ReaderNet(Arc::new(net.clone())));
    let bytes = printout_with(&store, one_flow(thread), &sources);
    assert_eq!(net.asked(), [PHOTO], "asked while the consent stood");
    assert!(drawn(&bytes).is_empty(), "drawn after the consent was gone");
    assert!(all_text(&open(&bytes)).contains(&squeeze(PHOTO_ALT)));
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
        let got = print_through(
            &store,
            job,
            &Paper::plain(),
            &printer,
            &Sources::default(),
            &chrono::Utc,
            now(),
        );
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
        &Sources::default(),
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
/// MAILO_SAMPLE_PDF=/path/sample-print.pdf cargo test -p mail-app -- --ignored \
///     write_a_sample_printout
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
