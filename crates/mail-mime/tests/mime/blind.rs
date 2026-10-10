//! The sender's copy names who was blind-copied (RFC 5322 §3.6.3); the transmitted bytes never do.

use mail_mime::{parse, with_blind};

const SENT: &str = "From: me@example.test\r\n\
To: Bea <bea@example.test>\r\n\
Cc: cara@example.test\r\n\
Subject: lunch\r\n\
\r\n\
Bcc: this line is the body\r\n";

fn rcpt(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn an_envelope_address_no_header_names_becomes_the_bcc_field() {
    let copy = with_blind(
        SENT.as_bytes(),
        &rcpt(&[
            "bea@example.test",
            "CARA@example.test",
            "dee@example.test",
            "eve@example.test",
        ]),
    );
    let text = String::from_utf8(copy.clone()).unwrap();
    assert!(
        text.contains("\r\nBcc: dee@example.test, eve@example.test\r\n"),
        "{text}"
    );
    // The field sits in the header: the body is untouched, byte for byte.
    assert!(
        text.ends_with("\r\n\r\nBcc: this line is the body\r\n"),
        "{text}"
    );
    let parsed = parse(&copy).unwrap();
    let blind: Vec<_> = parsed.bcc.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(blind, ["dee@example.test", "eve@example.test"]);
    let visible: Vec<_> = parsed
        .to
        .iter()
        .chain(&parsed.cc)
        .map(|a| a.email.as_str())
        .collect();
    assert_eq!(visible, ["bea@example.test", "cara@example.test"]);
}

#[test]
fn nothing_blind_means_the_same_bytes() {
    let rcpts = rcpt(&["bea@example.test", "cara@example.test", ""]);
    assert_eq!(with_blind(SENT.as_bytes(), &rcpts), SENT.as_bytes());
}

#[test]
fn a_message_that_already_names_its_bcc_is_not_given_a_second() {
    let once = with_blind(SENT.as_bytes(), &rcpt(&["dee@example.test"]));
    assert_eq!(with_blind(&once, &rcpt(&["dee@example.test"])), once);
}

#[test]
fn a_long_blind_list_folds_before_78_columns_and_still_reads_back() {
    let many: Vec<String> = (0..12)
        .map(|n| format!("recipient{n}@blind.example.test"))
        .collect();
    let copy = with_blind(SENT.as_bytes(), &many);
    let text = String::from_utf8(copy.clone()).unwrap();
    let head = text.split("\r\n\r\n").next().unwrap();
    for line in head.split("\r\n") {
        assert!(
            line.len() <= 78,
            "a header line of {} octets: {line}",
            line.len()
        );
    }
    let parsed = parse(&copy).unwrap();
    assert_eq!(parsed.bcc.len(), 12);
}
