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

/// `(name, charset, subject, extra headers)`. Every row's field is written in raw bytes of
/// `charset`, so none of them is UTF-8.
#[test]
fn a_raw_subject_is_read_in_its_charset() {
    let cases: &[(&str, &'static Encoding, &str, &str)] = &[
        (
            "big5, in the charset the message declares",
            BIG5,
            "會議記錄與下週行程",
            "MIME-Version: 1.0\r\nContent-Type: text/plain; charset=big5\r\n",
        ),
        (
            "gbk with no declaration is detected",
            GBK,
            "关于下周项目会议的安排和准备工作",
            "",
        ),
        (
            "shift_jis with no declaration is detected",
            SHIFT_JIS,
            "来週の会議の議事録について",
            "",
        ),
        (
            "koi8-r",
            KOI8_R,
            "Отчёт о встрече на прошлой неделе",
            "Content-Type: text/plain; charset=\"koi8-r\"\r\n",
        ),
        // A field with 8-bit bytes in it cannot be US-ASCII or UTF-8, whatever the part says.
        // Both mislabels are common: a template declares UTF-8, the server under it writes GBK.
        (
            "a us-ascii declaration the raw bytes contradict gives way to the detector",
            GBK,
            "关于下周项目会议的安排和准备工作",
            "Content-Type: text/plain; charset=us-ascii\r\n",
        ),
        (
            "a quoted US-ASCII declaration the raw bytes contradict gives way to the detector",
            GBK,
            "关于下周项目会议的安排和准备工作",
            "Content-Type: text/plain; charset=\"US-ASCII\"\r\n",
        ),
        (
            "a utf-8 declaration the raw bytes contradict gives way to the detector",
            GBK,
            "关于下周项目会议的安排和准备工作",
            "Content-Type: text/plain; charset=utf-8\r\n",
        ),
        (
            "a UTF8 declaration the raw bytes contradict gives way to the detector",
            GBK,
            "关于下周项目会议的安排和准备工作",
            "Content-Type: text/plain; charset=UTF8\r\n",
        ),
        (
            "ascii around the raw bytes is kept",
            GBK,
            "Re: [dev-list] 下周会议 agenda (v2)",
            "Content-Type: text/plain; charset=gb2312\r\n",
        ),
    ];
    for (name, charset, subject, extra) in cases {
        let raw = with_subject(charset, subject, extra);
        assert!(
            std::str::from_utf8(&raw).is_err(),
            "{name}: the fixture must be raw {}",
            charset.name()
        );
        assert_eq!(parse(&raw).unwrap().subject, *subject, "{name}");
    }
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
