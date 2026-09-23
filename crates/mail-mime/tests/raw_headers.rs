//! Header fields written in raw bytes of a legacy charset, not in UTF-8 or an encoded-word.
//!
//! Each message is assembled from an ASCII frame and a field encoded by `encoding_rs`, so the
//! bytes under test are exactly what an old server stores and nothing in this file is UTF-8
//! pretending to be something else.

use encoding_rs::{BIG5, Encoding, GBK, KOI8_R, SHIFT_JIS};
use mail_mime::parse;

/// A message whose `Subject` is `subject` written in `charset`, with `extra` headers above it.
fn with_subject(charset: &'static Encoding, subject: &str, extra: &str) -> Vec<u8> {
    let (bytes, _, unmappable) = charset.encode(subject);
    assert!(
        !unmappable,
        "{subject:?} is not writable in {}",
        charset.name()
    );
    let mut raw = b"From: ada@example.test\r\nTo: bea@example.test\r\n".to_vec();
    raw.extend_from_slice(extra.as_bytes());
    raw.extend_from_slice(b"Subject: ");
    raw.extend_from_slice(&bytes);
    raw.extend_from_slice(b"\r\n\r\nbody\r\n");
    raw
}

#[test]
fn a_big5_subject_is_read_in_the_charset_the_message_declares() {
    let raw = with_subject(
        BIG5,
        "會議記錄與下週行程",
        "MIME-Version: 1.0\r\nContent-Type: text/plain; charset=big5\r\n",
    );
    assert!(
        std::str::from_utf8(&raw).is_err(),
        "the fixture must be raw Big5"
    );
    assert_eq!(parse(&raw).unwrap().subject, "會議記錄與下週行程");
}

#[test]
fn a_gbk_subject_with_no_declaration_is_detected() {
    let raw = with_subject(GBK, "关于下周项目会议的安排和准备工作", "");
    assert_eq!(
        parse(&raw).unwrap().subject,
        "关于下周项目会议的安排和准备工作"
    );
}

#[test]
fn a_shift_jis_subject_is_read() {
    // Declared on the first text part of a multipart, not at the top: the top-level type is
    // `multipart/alternative`, which carries no charset of its own.
    let subject = "来週の会議の議事録について";
    let (bytes, _, _) = SHIFT_JIS.encode(subject);
    let mut raw = b"From: ada@example.test\r\nSubject: ".to_vec();
    raw.extend_from_slice(&bytes);
    raw.extend_from_slice(
        b"\r\nMIME-Version: 1.0\r\n\
          Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
          --b\r\nContent-Type: text/plain; charset=Shift_JIS\r\n\r\nhi\r\n\
          --b\r\nContent-Type: text/html; charset=Shift_JIS\r\n\r\n<p>hi</p>\r\n--b--\r\n",
    );
    assert_eq!(parse(&raw).unwrap().subject, subject);
}

#[test]
fn a_shift_jis_subject_with_no_declaration_is_detected() {
    let raw = with_subject(SHIFT_JIS, "来週の会議の議事録について", "");
    assert_eq!(parse(&raw).unwrap().subject, "来週の会議の議事録について");
}

#[test]
fn a_koi8_r_subject_is_read() {
    let raw = with_subject(
        KOI8_R,
        "Отчёт о встрече на прошлой неделе",
        "Content-Type: text/plain; charset=\"koi8-r\"\r\n",
    );
    assert_eq!(
        parse(&raw).unwrap().subject,
        "Отчёт о встрече на прошлой неделе"
    );
}

#[test]
fn a_declaration_the_raw_bytes_contradict_gives_way_to_the_detector() {
    // A field with 8-bit bytes in it cannot be US-ASCII or UTF-8, whatever the part says. Both
    // mislabels are common: a template declares UTF-8, the server under it writes GBK.
    const DECLARED: &[&str] = &["us-ascii", "\"US-ASCII\"", "utf-8", "UTF8"];
    for charset in DECLARED {
        let raw = with_subject(
            GBK,
            "关于下周项目会议的安排和准备工作",
            &format!("Content-Type: text/plain; charset={charset}\r\n"),
        );
        assert_eq!(
            parse(&raw).unwrap().subject,
            "关于下周项目会议的安排和准备工作",
            "declared {charset}"
        );
    }
}

#[test]
fn a_declared_charset_that_cannot_read_the_field_gives_way_to_the_detector() {
    // Declared Shift_JIS over GBK bytes: `0xFE` is no Shift_JIS byte at all, so
    // the declaration fails on this field and is not believed for it.
    let (mut bytes, _, _) = GBK.encode("关于下周项目会议的安排和准备工作");
    bytes.to_mut().splice(0..0, [0xFE, 0x40]);
    let mut raw = b"From: ada@example.test\r\nSubject: ".to_vec();
    raw.extend_from_slice(&bytes);
    raw.extend_from_slice(b"\r\nContent-Type: text/plain; charset=shift_jis\r\n\r\nbody\r\n");
    assert!(
        SHIFT_JIS
            .decode_without_bom_handling_and_without_replacement(&bytes)
            .is_none(),
        "the fixture must be undecodable as Shift_JIS"
    );
    let subject = parse(&raw).unwrap().subject;
    assert!(
        subject.ends_with("关于下周项目会议的安排和准备工作"),
        "{subject:?}"
    );
}

#[test]
fn an_encoded_word_in_a_multi_byte_charset_is_decoded() {
    // `=?gb2312?B?…?=` and `=?big5?B?…?=` are how most CJK mail writes its subject, and were
    // decoded as UTF-8 — that is, lost — until the parser was built with every charset.
    const CASES: &[(&str, &str)] = &[
        ("=?gb2312?B?udjT2s/C1ty1xLvh0uk=?=", "关于下周的会议"),
        ("=?big5?B?t3zEs7BPv/0=?=", "會議記錄"),
    ];
    for (word, expected) in CASES {
        let raw = format!("From: ada@example.test\r\nSubject: {word}\r\n\r\nbody\r\n");
        assert_eq!(parse(raw.as_bytes()).unwrap().subject, *expected, "{word}");
    }
}

#[test]
fn ascii_around_the_raw_bytes_is_kept() {
    let raw = with_subject(
        GBK,
        "Re: [dev-list] 下周会议 agenda (v2)",
        "Content-Type: text/plain; charset=gb2312\r\n",
    );
    assert_eq!(
        parse(&raw).unwrap().subject,
        "Re: [dev-list] 下周会议 agenda (v2)"
    );
}

#[test]
fn a_raw_display_name_is_decoded_and_the_address_survives() {
    let (name, _, _) = BIG5.encode("王小明");
    let mut raw = b"From: ".to_vec();
    raw.extend_from_slice(&name);
    raw.extend_from_slice(
        b" <wang@example.test>\r\nSubject: hi\r\n\
          Content-Type: text/plain; charset=big5\r\n\r\nbody\r\n",
    );
    let from = parse(&raw).unwrap().from.unwrap();
    assert_eq!(from.name.as_deref(), Some("王小明"));
    assert_eq!(from.email, "wang@example.test");
}

#[test]
fn valid_utf8_headers_are_untouched() {
    let raw = "From: 王小明 <wang@example.test>\r\n\
               Subject: 會議記錄 — café\r\n\
               Content-Type: text/plain; charset=big5\r\n\r\nbody\r\n";
    let parsed = parse(raw.as_bytes()).unwrap();
    assert_eq!(parsed.subject, "會議記錄 — café");
    assert_eq!(parsed.from.unwrap().name.as_deref(), Some("王小明"));
}

#[test]
fn an_encoded_word_beside_a_raw_field_keeps_its_own_charset() {
    // The subject is an RFC 2047 encoded-word in UTF-8; the display name is raw Big5. The
    // declared Big5 applies to the raw field only.
    let (name, _, _) = BIG5.encode("王小明");
    let mut raw = b"From: ".to_vec();
    raw.extend_from_slice(&name);
    raw.extend_from_slice(
        b" <wang@example.test>\r\n\
          Subject: =?UTF-8?B?5pyD6K2w6KiY6YyE?=\r\n\
          Content-Type: text/plain; charset=big5\r\n\r\nbody\r\n",
    );
    let parsed = parse(&raw).unwrap();
    assert_eq!(parsed.subject, "會議記錄");
    assert_eq!(parsed.from.unwrap().name.as_deref(), Some("王小明"));
}

#[test]
fn a_folded_raw_subject_is_decoded_across_its_lines() {
    let (first, _, _) = GBK.encode("关于下周项目会议的");
    let (second, _, _) = GBK.encode("安排和准备工作");
    let mut raw = b"From: ada@example.test\r\nSubject: ".to_vec();
    raw.extend_from_slice(&first);
    raw.extend_from_slice(b"\r\n ");
    raw.extend_from_slice(&second);
    raw.extend_from_slice(b"\r\nContent-Type: text/plain; charset=gbk\r\n\r\nbody\r\n");
    assert_eq!(
        parse(&raw).unwrap().subject,
        "关于下周项目会议的 安排和准备工作"
    );
}

#[test]
fn the_body_is_not_rewritten() {
    // A Big5 body under a Big5 declaration: decoded once, by the body's own charset, and not a
    // second time by anything the header pass did.
    let (subject, _, _) = BIG5.encode("會議");
    let (body, _, _) = BIG5.encode("內容");
    let mut raw = b"From: ada@example.test\r\nSubject: ".to_vec();
    raw.extend_from_slice(&subject);
    raw.extend_from_slice(b"\r\nContent-Type: text/plain; charset=big5\r\n\r\n");
    raw.extend_from_slice(&body);
    raw.extend_from_slice(b"\r\n");
    let parsed = parse(&raw).unwrap();
    assert_eq!(parsed.subject, "會議");
    assert_eq!(parsed.text.as_deref().map(str::trim_end), Some("內容"));
}
