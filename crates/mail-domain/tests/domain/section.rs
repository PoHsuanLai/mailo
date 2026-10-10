use mail_domain::{ParseSectionError, Section};

#[test]
fn every_section_a_server_names_round_trips_as_text_and_as_json() {
    for text in [
        "",
        "1",
        "1.3",
        "12.4.1",
        "HEADER",
        "TEXT",
        "2.MIME",
        "1.1.HEADER",
        "3.TEXT",
    ] {
        let section: Section = text.parse().unwrap_or_else(|e| panic!("{text:?}: {e}"));
        assert_eq!(section.to_string(), text);
        assert_eq!(section.as_str(), text);
        let json = serde_json::to_string(&section).unwrap();
        assert_eq!(
            json,
            serde_json::to_string(text).unwrap(),
            "stored bytes are the text"
        );
        assert_eq!(serde_json::from_str::<Section>(&json).unwrap(), section);
    }
}

#[test]
fn text_that_is_not_a_section_is_refused_by_parsing_and_by_deserializing() {
    for bad in [
        "1] BODY[",
        "0",
        "1..2",
        "1.MIME.MIME",
        "2 FLAGS",
        ".MIME",
        "MIME",
        "1.",
        "a",
        "1.0",
        "1234567890",
        "header",
    ] {
        assert_eq!(
            bad.parse::<Section>(),
            Err(ParseSectionError(bad.to_owned())),
            "{bad:?}"
        );
        let json = serde_json::to_string(bad).unwrap();
        assert!(serde_json::from_str::<Section>(&json).is_err(), "{bad:?}");
    }
}

#[test]
fn a_section_is_built_from_a_path_and_names_its_headers() {
    assert_eq!(Section::of(&[1, 3]).unwrap(), "1.3");
    assert_eq!(Section::of(&[]), None);
    assert_eq!(Section::of(&[1, 0]), None);
    assert_eq!(Section::of(&[2]).unwrap().mime().unwrap(), "2.MIME");
    assert_eq!(Section::root().mime(), None);
    assert_eq!(Section::header().mime(), None);
    assert_eq!("2.MIME".parse::<Section>().unwrap().mime(), None);
    assert!(Section::root().is_root());
}
