use super::recent::{KEPT, load, remember, save};
use super::{Group, TABLE, Trigger, all, find, in_group, search, trigger, words};

#[test]
fn every_line_of_the_table_is_an_emoji_under_a_known_group() {
    let lines = TABLE
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("= "))
        .count();
    assert_eq!(all().len(), lines, "a line of the table was skipped");
    let titles: Vec<&str> = TABLE
        .lines()
        .filter_map(|line| line.strip_prefix("= "))
        .collect();
    let known: Vec<&str> = Group::ALL.iter().map(|group| group.title()).collect();
    assert_eq!(titles, known, "the table's groups, in order");
    for group in Group::ALL {
        assert!(
            in_group(group).any(|emoji| emoji.glyph == group.face()),
            "{group:?}'s tab face is one of its own"
        );
    }
    for emoji in all() {
        assert!(!emoji.name.is_empty(), "{} has no name", emoji.glyph);
        assert!(
            !emoji
                .glyph
                .chars()
                .any(|ch| ('\u{1F3FB}'..='\u{1F3FF}').contains(&ch)),
            "{} carries a skin tone",
            emoji.name
        );
    }
}

#[test]
fn a_colon_offers_emoji_only_where_a_name_is_being_typed() {
    let cases: &[(&str, Option<(usize, &str)>)] = &[
        (":smi", Some((0, "smi"))),
        ("Hello :smi", Some((6, "smi"))),
        ("Hello :thumbs_up", Some((6, "thumbs_up"))),
        ("ok :+1", Some((3, "+1"))),
        ("a\u{a0}:heart-eyes", Some((2, "heart-eyes"))),
        ("注音 :cat", Some((3, "cat"))),
        // Too short to be a name yet: ordinary writing.
        (":", None),
        (":s", None),
        // The colon ends a word, or sits inside one.
        ("Note:", None),
        ("Note:smi", None),
        ("at 10:30", None),
        ("https://example", None),
        // A space, or a mark that is no part of a name, ends it.
        (":smi le", None),
        (":)", None),
        (":-(", None),
        (":smile:", None),
        // The last colon is the one being typed.
        (":smile: :ca", Some((8, "ca"))),
        (":abcdefghijklmnopqrstuvwxyzabcdefg", None),
    ];
    for (before, expected) in cases {
        let found = trigger(before);
        let expected = expected.map(|(at, query)| Trigger { at, query });
        assert_eq!(found, expected, "{before:?}");
    }
}

#[test]
fn a_name_finds_emoji_by_the_start_of_its_words_best_first() {
    let first = |query: &str| search(query).first().map(|emoji| emoji.glyph);
    let cases: &[(&str, Option<&str>)] = &[
        // The name starts with it: the table's first such.
        ("smi", Some("😊")),
        ("SMI", Some("😊")),
        // The whole name wins over a name that only starts with it.
        ("smiling face", Some("☺️")),
        ("thumbs_up", Some("👍")),
        ("thumbs up", Some("👍")),
        // Every word starts a word of the name.
        ("tears joy", Some("😂")),
        ("heart-eyes", Some("😍")),
        // A whole word, of the name or a keyword, before a name that only starts with it.
        ("lol", Some("😄")),
        ("joy", Some("😂")),
        ("+1", Some("👍")),
        ("flag japan", Some("🇯🇵")),
        // Word starts only: `ear` is the ear, not the start of every bear and heart.
        ("ear", Some("👂")),
        ("zzzqqq", None),
        ("", None),
        ("__", None),
    ];
    for (query, expected) in cases {
        assert_eq!(first(query), *expected, "{query:?}");
    }
    for emoji in search("ear") {
        let lower = emoji.name.to_lowercase();
        assert!(
            words(&lower)
                .chain(words(emoji.keywords))
                .any(|word| word.starts_with("ear")),
            "{} was matched from the middle of a word",
            emoji.name
        );
    }
    let smi = search("smi");
    let order: Vec<&str> = smi.iter().take(3).map(|emoji| emoji.name).collect();
    assert!(
        order.iter().all(|name| name.starts_with("smi")),
        "names that start with the query come first: {order:?}"
    );
    assert!(
        smi.iter().any(|emoji| emoji.glyph == "😁"),
        "a name with a word that starts with it is found too"
    );
}

#[test]
fn recent_emoji_are_newest_first_without_repeats_and_kept() {
    let dir = tempfile::tempdir().unwrap_or_else(|why| panic!("{why}"));
    assert!(
        load(dir.path()).is_empty(),
        "nothing until something is picked"
    );
    let smile = find("😊").unwrap_or_else(|| panic!("😊 is in the table"));
    let cat = find("🐱").unwrap_or_else(|| panic!("🐱 is in the table"));
    let once = remember(&[], smile);
    let twice = remember(&once, cat);
    let again = remember(&twice, smile);
    assert_eq!(
        again,
        vec![smile, cat],
        "picked again, it moves to the front"
    );
    save(dir.path(), &again).unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(load(dir.path()), vec![smile, cat]);

    let many: Vec<_> = all().iter().take(KEPT + 5).collect();
    let full = many
        .iter()
        .fold(Vec::new(), |recent, emoji| remember(&recent, emoji));
    assert_eq!(full.len(), KEPT, "the oldest go past the limit");
    assert_eq!(full[0], many[KEPT + 4], "the newest first");

    std::fs::write(
        dir.path().join(super::recent::FILE_NAME),
        r#"{"recent":["🐱","not an emoji","🐱","😊"]}"#,
    )
    .unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(
        load(dir.path()),
        vec![cat, smile],
        "what the table lacks, and repeats, are dropped"
    );
    std::fs::write(dir.path().join(super::recent::FILE_NAME), b"not json")
        .unwrap_or_else(|why| panic!("{why}"));
    assert!(load(dir.path()).is_empty(), "an unreadable file is nothing");
}
