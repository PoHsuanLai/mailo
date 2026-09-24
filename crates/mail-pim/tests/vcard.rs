//! vCards as the programs people export from write them, read and written back as 4.0.
//!
//! The samples are written for these tests in the shapes RFC 6350, RFC 2426 and the vCard 2.1
//! specification describe, including the parts of those shapes phone and webmail exports lean
//! on: grouped lines, bare 2.1 parameters, quoted-printable names, inline photos.

use mail_pim::line::ContentLine;
use mail_pim::vcard::{self, Card, Email, Name, Phone, Version};

const V21: &str = "BEGIN:VCARD\r\n\
VERSION:2.1\r\n\
N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:M=C3=BCller;J=C3=BCrgen;;;\r\n\
FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:J=C3=BCrgen M=C3=BCller\r\n\
TEL;CELL;PREF:+49 170 1234567\r\n\
TEL;WORK;VOICE:+49 30 555\r\n\
EMAIL;INTERNET;HOME:juergen@example.test\r\n\
EMAIL;INTERNET;WORK;PREF:jm@work.example.test\r\n\
NOTE;ENCODING=QUOTED-PRINTABLE:Met at the=0D=0Aconference in 20=\r\n\
19\r\n\
PHOTO;ENCODING=BASE64;TYPE=JPEG:\r\n\
\x20/9j/4AAQSkZJRg\r\n\
\x20==\r\n\
\r\n\
END:VCARD\r\n";

const V30: &str = "BEGIN:VCARD\n\
VERSION:3.0\n\
PRODID:-//Example//Exporter//EN\n\
N:Lovelace;Ada;Augusta;Countess;\n\
FN:Ada Lovelace\n\
ORG:Analytical Engines\\, Ltd.;Research\n\
item1.EMAIL;type=INTERNET;type=pref:ada@example.test\n\
item1.X-ABLabel:_$!<Other>!$_\n\
EMAIL;TYPE=INTERNET,WORK:ada@engines.example.test\n\
TEL;TYPE=CELL:+44 20 7946 0000\n\
BDAY:1815-12-10\n\
NOTE:Wrote the first program\\; see notes\\nfor details.\n\
UID:ada-1815\n\
REV:2024-01-02T03:04:05Z\n\
END:VCARD\n";

const V40: &str = "BEGIN:VCARD\r\n\
VERSION:4.0\r\n\
UID:urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1\r\n\
FN:Grace Hopper\r\n\
N:Hopper;Grace;Brewster Murray;Rear Admiral;\r\n\
EMAIL;TYPE=work;PREF=1:grace@navy.example.test\r\n\
EMAIL;TYPE=home:grace@home.example.test\r\n\
TEL;VALUE=uri;TYPE=\"voice,cell\":tel:+1-555-555-0100\r\n\
PHOTO:https://photos.example.test/grace.jpg\r\n\
ADR;TYPE=work:;;1 Main St;Arlington;VA;22201;USA\r\n\
END:VCARD\r\n";

#[test]
fn a_version_2_1_card_is_decoded_from_quoted_printable_in_its_charset() {
    let cards = vcard::parse(V21);
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card.version, Version::V2_1);
    assert_eq!(card.formatted_name.as_deref(), Some("Jürgen Müller"));
    assert_eq!(
        card.name
            .as_ref()
            .map(|n| (n.family.as_str(), n.given.as_str())),
        Some(("Müller", "Jürgen"))
    );
    assert_eq!(
        card.emails,
        [
            Email {
                address: "juergen@example.test".into(),
                kinds: vec!["internet".into(), "home".into()],
                pref: None,
            },
            Email {
                address: "jm@work.example.test".into(),
                kinds: vec!["internet".into(), "work".into()],
                pref: Some(1),
            },
        ]
    );
    assert_eq!(card.phones[0].pref, Some(1));
    assert_eq!(card.phones[1].kinds, ["work", "voice"]);
    assert_eq!(card.note.as_deref(), Some("Met at the\nconference in 2019"));
    assert_eq!(
        card.photo.as_deref(),
        Some("data:image/jpeg;base64,/9j/4AAQSkZJRg==")
    );
    assert_eq!(
        card.addresses_by_preference(),
        ["jm@work.example.test", "juergen@example.test"]
    );
}

#[test]
fn a_version_3_0_card_keeps_its_groups_escapes_and_unread_properties() {
    let card = &vcard::parse(V30)[0];
    assert_eq!(card.version, Version::V3_0);
    assert_eq!(card.uid.as_deref(), Some("ada-1815"));
    assert_eq!(card.org, ["Analytical Engines, Ltd.", "Research"]);
    assert_eq!(
        card.note.as_deref(),
        Some("Wrote the first program; see notes\nfor details.")
    );
    assert_eq!(card.emails[0].address, "ada@example.test");
    assert_eq!(card.emails[0].pref, Some(1));
    assert_eq!(card.emails[1].kinds, ["internet", "work"]);
    assert_eq!(card.revision.as_deref(), Some("2024-01-02T03:04:05Z"));
    let other: Vec<&str> = card.other.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(
        other,
        ["X-ABLABEL", "BDAY"],
        "PRODID is dropped, the rest kept"
    );
    assert_eq!(card.other[0].group.as_deref(), Some("item1"));
}

#[test]
fn a_version_4_0_card_reads_uris_and_quoted_type_lists() {
    let card = &vcard::parse(V40)[0];
    assert_eq!(card.version, Version::V4_0);
    assert_eq!(card.phones[0].number, "+1-555-555-0100");
    assert_eq!(card.phones[0].kinds, ["voice", "cell"]);
    assert_eq!(
        card.photo.as_deref(),
        Some("https://photos.example.test/grace.jpg")
    );
    assert_eq!(card.display_name().as_deref(), Some("Grace Hopper"));
}

#[test]
fn every_version_reads_back_unchanged_after_being_written_as_4_0() {
    for (label, text) in [("2.1", V21), ("3.0", V30), ("4.0", V40)] {
        let read = vcard::parse(text).remove(0);
        let written = vcard::write(&read);
        assert!(
            written.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"),
            "{label}"
        );
        let again = vcard::parse(&written).remove(0);
        // Everything but the version it was read as, which is now 4.0.
        assert_eq!(
            Card {
                version: read.version,
                ..again.clone()
            },
            Card {
                emails: read
                    .emails
                    .iter()
                    .map(|e| Email {
                        kinds: e
                            .kinds
                            .iter()
                            .filter(|k| *k != "internet")
                            .cloned()
                            .collect(),
                        ..e.clone()
                    })
                    .collect(),
                ..read.clone()
            },
            "{label}"
        );
        // And writing that is a fixed point.
        assert_eq!(vcard::write(&again), written, "{label}");
    }
}

#[test]
fn a_card_built_here_round_trips_through_text() {
    let card = Card {
        uid: Some("u-1".into()),
        formatted_name: Some("Ｚoë \"Zed\" O'Neil, Jr.".into()),
        name: Some(Name {
            family: "O'Neil".into(),
            given: "Zoë".into(),
            suffixes: "Jr.".into(),
            ..Name::default()
        }),
        emails: vec![Email {
            address: "zoe@example.test".into(),
            kinds: vec!["home".into()],
            pref: Some(3),
        }],
        phones: vec![Phone {
            number: "+1 (555) 010-0199".into(),
            kinds: vec!["cell".into()],
            pref: None,
        }],
        org: vec!["Semi;colon, Inc.".into()],
        note: Some("line one\nline two, with \\ backslash".into()),
        other: vec![ContentLine::new("X-CUSTOM", "kept").with("X-P", &["a:b"])],
        ..Card::new()
    };
    let text = vcard::write(&card);
    assert_eq!(vcard::parse(&text), [card]);
}

#[test]
fn a_card_with_no_name_is_written_with_its_address_as_fn() {
    let card = Card {
        emails: vec![Email {
            address: "nobody@example.test".into(),
            kinds: vec![],
            pref: None,
        }],
        ..Card::new()
    };
    assert!(vcard::write(&card).contains("\r\nFN:nobody@example.test\r\n"));
}

#[test]
fn several_cards_in_one_file_are_all_read_and_all_written() {
    let joined = format!("{V21}{V30}{V40}");
    let cards = vcard::parse(&joined);
    assert_eq!(cards.len(), 3);
    assert_eq!(vcard::parse(&vcard::write_all(&cards)).len(), 3);
}

#[test]
fn a_nested_card_and_stray_text_are_not_contacts() {
    let text = "junk before\r\nBEGIN:VCARD\r\nVERSION:2.1\r\nFN:Outer\r\n\
AGENT:\r\nBEGIN:VCARD\r\nFN:Inner\r\nEND:VCARD\r\nEMAIL:outer@example.test\r\n\
END:VCARD\r\nmore junk\r\n";
    let cards = vcard::parse(text);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].formatted_name.as_deref(), Some("Outer"));
    assert_eq!(cards[0].emails[0].address, "outer@example.test");
}

#[test]
fn a_file_in_a_legacy_charset_is_read_rather_than_refused() {
    let mut bytes = b"BEGIN:VCARD\r\nVERSION:2.1\r\nFN:Ren".to_vec();
    bytes.push(0xE9); // é in Windows-1252
    bytes.extend_from_slice(b"e\r\nEND:VCARD\r\n");
    let cards = vcard::parse_bytes(&bytes);
    assert_eq!(cards[0].formatted_name.as_deref(), Some("Renée"));
}

#[test]
fn a_card_cut_off_by_the_end_of_the_file_keeps_what_it_had() {
    let cards = vcard::parse("BEGIN:VCARD\nFN:Half\nEMAIL:half@example.test\n");
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].emails[0].address, "half@example.test");
}
