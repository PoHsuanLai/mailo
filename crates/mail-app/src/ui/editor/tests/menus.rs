//! The slash menu, `@` mentions, and the attachment guard.

use super::*;

#[test]
fn attachment_detector_in_six_languages() {
    let positives = [
        ("en", "see the attachment"),
        ("en", "I have attached the slides"),
        ("en, a guard not a judge", "I didn't attach it"),
        ("en", "file enclosed."),
        ("zh", "请看附件"),
        ("zh-Hant", "詳見附件。"),
        ("ja", "ファイルを添付しました"),
        ("fr", "le document ci-joint"),
        ("fr", "ci jointe au message"),
        ("es", "el archivo adjunto"),
        ("de", "anbei die Unterlagen"),
        ("de", "siehe Anhang"),
    ];
    for (language, text) in positives {
        assert!(mentions_attachment(text), "{language}: {text}");
    }
    let negatives = [
        ("en", "the cat sat on the mat"),
        ("en, not a whole word", "an unattached cable"),
        ("en", "I will send the notes tomorrow"),
        ("zh", "我明天寄給你"),
        ("ja", "明日送ります"),
        ("fr", "le document est prêt"),
        ("es", "te lo mando mañana"),
        ("de", "die Unterlagen folgen"),
    ];
    for (language, text) in negatives {
        assert!(!mentions_attachment(text), "{language}: {text}");
    }

    let mut doc = Doc::from_text("see the attachment");
    assert!(missing_attachment(&doc));
    doc.nodes
        .push(Node::Object(Object::Attachment(AttachmentRef::new(
            "a.pdf",
        ))));
    assert!(
        !missing_attachment(&doc),
        "an attachment object satisfies the guard"
    );
}

#[test]
fn a_mention_resolves_and_joins_cc_once() {
    let anna = Person {
        name: "Anna".into(),
        address: "anna@example.com".into(),
    };
    let joanna = Person {
        name: "Joanna".into(),
        address: "jo@example.com".into(),
    };
    let people = vec![anna.clone(), joanna.clone()];
    let names = |query: &str| -> Vec<String> {
        resolve(query, &people)
            .into_iter()
            .map(|person| person.name.clone())
            .collect()
    };
    assert_eq!(
        names("ann"),
        ["Anna"],
        "a prefix of the name, not a substring"
    );
    assert_eq!(names("@Jo"), ["Joanna"]);
    assert_eq!(
        names("anna@"),
        ["Anna"],
        "the address, when the name does not match"
    );
    assert_eq!(names("").len(), 2);

    assert_eq!(joins_cc(&anna, &[], &[]), Some(&anna));
    assert_eq!(
        joins_cc(&anna, &[], std::slice::from_ref(&anna)),
        None,
        "already on Cc"
    );
    let shouting = Person {
        name: "Anna".into(),
        address: "ANNA@example.com".into(),
    };
    assert_eq!(joins_cc(&anna, &[shouting], &[]), None, "already on To");
}

#[test]
fn slash_filter_order() {
    let names =
        |query: &str| -> Vec<&str> { filter(query).into_iter().map(|item| item.name).collect() };
    assert_eq!(
        names("he")[..3],
        ["Heading 1", "Heading 2", "Heading 3"],
        "he: names first"
    );
    assert_eq!(names("tab")[0], "Table");
    assert_eq!(names("hr")[0], "Divider", "a keyword finds its row");
    assert_eq!(names("dvdr")[0], "Divider", "fuzzy, last");
    assert!(names("zzzz").is_empty());
    assert_eq!(
        filter("").len(),
        catalog().len(),
        "an empty query lists everything"
    );
    assert!(
        filter("").windows(2).all(|pair| {
            let at = |name| catalog().iter().position(|item| item.name == name);
            at(pair[0].name) < at(pair[1].name)
        }),
        "in catalog order"
    );

    let turn = turn_into();
    assert!(
        turn.iter()
            .all(|item| matches!(item.action, Action::Turn(_)))
    );
    assert_eq!(
        turn.len(),
        9,
        "Text, three headings, three lists, quote, code"
    );
    let every_item_is_described = catalog()
        .iter()
        .all(|item| !item.help.is_empty() && !item.icon.is_empty() && !item.keywords.is_empty());
    assert!(every_item_is_described);
}
