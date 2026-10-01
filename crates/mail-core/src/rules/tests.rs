use super::server::instant;
use super::*;

#[test]
fn a_date_is_the_start_of_that_day_in_the_readers_zone() {
    let taipei = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    let cases = [
        ("2026-10-08", "2026-10-07T16:00:00+00:00"),
        ("2026-10-08T09:00", "2026-10-08T01:00:00+00:00"),
        ("2026-10-08 09:30", "2026-10-08T01:30:00+00:00"),
    ];
    for (text, want) in cases {
        assert_eq!(instant(text, &taipei).unwrap().to_rfc3339(), want, "{text}");
    }
    assert!(instant("next week", &taipei).is_err());
}

#[test]
fn a_rule_is_listed_in_the_words_it_was_written_in() {
    let index: Vec<(String, LabelId)> = vec![("Money".into(), LabelId::generate())];
    let label_id = index[0].1;
    let filter = crate::query::parse_with(
        "from:bank.example -is:read label:money \"due date\"",
        &chrono::Utc,
        &crate::query::named(&index),
    );
    let name = |id: LabelId| {
        if id == label_id {
            "Money".to_owned()
        } else {
            "?".to_owned()
        }
    };
    assert_eq!(
        condition(&filter, &name),
        "from:bank.example -is:read label:Money \"due date\""
    );
}
