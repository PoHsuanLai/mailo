//! Rebuilding a large message from its sections, with its attachments left on the server.

use mail_domain::PartTree;
use mail_mime::{left_on_server, parse, parse_reconstructed, reconstruct, sections_for};
use std::collections::HashMap;

/// A report: plain and HTML alternatives, and a PDF beside them. Built from the same pieces an
/// IMAP server hands out as sections, so the original and the sections cannot disagree.
struct Report {
    header: &'static str,
    alt_mime: &'static str,
    plain_mime: &'static str,
    plain: &'static str,
    html_mime: &'static str,
    html: &'static str,
    pdf_mime: String,
    pdf: &'static str,
}

impl Report {
    fn new(pdf_extra_header: &str) -> Self {
        Report {
            header: "From: Ada <ada@example.test>\r\nTo: bob@example.test\r\nSubject: The report\r\nMessage-ID: <r@example.test>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n",
            alt_mime: "Content-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n",
            plain_mime: "Content-Type: text/plain; charset=utf-8\r\n\r\n",
            plain: "The numbers are attached.",
            html_mime: "Content-Type: text/html; charset=utf-8\r\n\r\n",
            html: "<p>The numbers are attached.</p>",
            pdf_mime: format!(
                "Content-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\n{pdf_extra_header}Content-Transfer-Encoding: base64\r\n\r\n"
            ),
            pdf: "JVBERi0xLjQKJcOkw7zDtsOfCjIgMCBvYmoKPDwvTGVuZ3RoIDMgMCBSPj4Kc3RyZWFtCg==",
        }
    }

    /// The whole message, as the server holds it.
    fn original(&self) -> Vec<u8> {
        format!(
            "{}--mix\r\n{}--alt\r\n{}{}\r\n--alt\r\n{}{}\r\n--alt--\r\n\r\n--mix\r\n{}{}\r\n--mix--\r\n",
            self.header,
            self.alt_mime,
            self.plain_mime,
            self.plain,
            self.html_mime,
            self.html,
            self.pdf_mime,
            self.pdf
        )
        .into_bytes()
    }

    fn tree(&self) -> PartTree {
        let leaf = |section: &str, mime: &str, octets: usize, attachment| PartTree::Leaf {
            section: section.to_owned(),
            mime: mime.to_owned(),
            octets: octets as u64,
            attachment,
        };
        PartTree::Multipart {
            section: String::new(),
            subtype: "mixed".to_owned(),
            boundary: "mix".to_owned(),
            parts: vec![
                PartTree::Multipart {
                    section: "1".to_owned(),
                    subtype: "alternative".to_owned(),
                    boundary: "alt".to_owned(),
                    parts: vec![
                        leaf("1.1", "text/plain", self.plain.len(), false),
                        leaf("1.2", "text/html", self.html.len(), false),
                    ],
                },
                leaf("2", "application/pdf", self.pdf.len(), true),
            ],
        }
    }

    /// What the server answers for each section name.
    fn server(&self) -> HashMap<String, Vec<u8>> {
        [
            ("HEADER", self.header.to_owned()),
            ("1.MIME", self.alt_mime.to_owned()),
            ("1.1.MIME", self.plain_mime.to_owned()),
            ("1.1", self.plain.to_owned()),
            ("1.2.MIME", self.html_mime.to_owned()),
            ("1.2", self.html.to_owned()),
            ("2.MIME", self.pdf_mime.clone()),
            ("2", self.pdf.to_owned()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.into_bytes()))
        .collect()
    }

    /// The sections `sections_for` asks for, answered.
    fn fetch(&self, keep: &dyn Fn(&PartTree) -> bool) -> HashMap<String, Vec<u8>> {
        let server = self.server();
        sections_for(&self.tree(), keep)
            .unwrap()
            .into_iter()
            .map(|s| (s.clone(), server[&s].clone()))
            .collect()
    }
}

fn not_attachments(node: &PartTree) -> bool {
    matches!(
        node,
        PartTree::Leaf {
            attachment: false,
            ..
        }
    )
}

/// Byte for byte, given one convention: a nested multipart's closing delimiter is followed by a
/// line break before the next outer delimiter, as `original` writes it. A message without that
/// line break comes back with it, which MIME reads as an empty epilogue — the same message.
#[test]
fn with_every_part_fetched_the_message_comes_back_byte_for_byte() {
    let report = Report::new("");
    let rebuilt = reconstruct(&report.tree(), &report.fetch(&|_| true)).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&rebuilt),
        String::from_utf8_lossy(&report.original())
    );
}

#[test]
fn only_the_sections_needed_are_asked_for() {
    let report = Report::new("");
    assert_eq!(
        sections_for(&report.tree(), &not_attachments).unwrap(),
        [
            "HEADER", "1.MIME", "1.1.MIME", "1.1", "1.2.MIME", "1.2", "2.MIME"
        ],
        "the PDF's content is the one thing not fetched"
    );
}

#[test]
fn an_attachment_left_behind_is_listed_and_the_rest_reads_as_before() {
    let report = Report::new("");
    let rebuilt = reconstruct(&report.tree(), &report.fetch(&not_attachments)).unwrap();
    let whole = parse(&report.original()).unwrap();
    let partial = parse_reconstructed(&rebuilt).unwrap();

    assert_eq!(partial.subject, whole.subject);
    assert_eq!(partial.text, whole.text);
    assert_eq!(partial.html, whole.html);
    assert_eq!(partial.attachments.len(), 1);
    let pdf = &partial.attachments[0];
    assert_eq!(pdf.name, "report.pdf");
    assert_eq!(pdf.mime, "application/pdf");
    assert!(pdf.bytes.is_empty(), "nothing was downloaded");
    let remote = pdf.remote.as_ref().expect("marked as left on the server");
    assert_eq!(remote.section, "2");
    assert_eq!(remote.octets, report.pdf.len() as u64);

    assert!(
        whole.attachments[0].remote.is_none(),
        "a whole message has nothing remote"
    );
}

/// A sender cannot mark their own attachment as remote, and have us fetch some other section
/// when it is opened, or show it as not downloaded when it was.
#[test]
fn a_marker_in_the_message_itself_is_removed_and_never_believed() {
    let report = Report::new("X-Mailo-Remote-Section: 1.1\r\nx-mailo-remote-octets:\r\n 9\r\n");
    let rebuilt = reconstruct(&report.tree(), &report.fetch(&|_| true)).unwrap();
    let text = String::from_utf8_lossy(&rebuilt).to_ascii_lowercase();
    assert!(!text.contains("x-mailo-remote"), "{text}");

    let parsed = parse_reconstructed(&rebuilt).unwrap();
    assert_eq!(parsed.attachments[0].remote, None);
    assert!(!parsed.attachments[0].bytes.is_empty());
}

/// What the source view and forward-as-attachment ask before treating stored bytes as the
/// message as sent.
#[test]
fn a_message_rebuilt_with_a_part_left_behind_says_so_and_a_whole_one_does_not() {
    let report = Report::new("");
    let partial = reconstruct(&report.tree(), &report.fetch(&not_attachments)).unwrap();
    let whole = reconstruct(&report.tree(), &report.fetch(&|_| true)).unwrap();
    assert!(left_on_server(&partial));
    assert!(!left_on_server(&report.original()));
    assert!(!left_on_server(&whole), "nothing was left behind");
    assert!(!left_on_server(b"not a message"));
}

#[test]
fn ordinary_parsing_ignores_the_markers() {
    let report = Report::new("");
    let rebuilt = reconstruct(&report.tree(), &report.fetch(&not_attachments)).unwrap();
    assert_eq!(parse(&rebuilt).unwrap().attachments[0].remote, None);
}

#[test]
fn a_message_that_is_not_multipart_is_fetched_whole() {
    let leaf = PartTree::Leaf {
        section: "1".to_owned(),
        mime: "text/plain".to_owned(),
        octets: 9,
        attachment: false,
    };
    assert_eq!(sections_for(&leaf, &|_| true), None);
    assert_eq!(reconstruct(&leaf, &HashMap::new()), None);
}

#[test]
fn a_missing_header_section_means_fetch_it_whole() {
    let report = Report::new("");
    let mut fetched = report.fetch(&not_attachments);
    fetched.remove("1.2.MIME");
    assert_eq!(reconstruct(&report.tree(), &fetched), None);
}
