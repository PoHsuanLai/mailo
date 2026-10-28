//! A rule's condition as rows built from menus: what is looked at (From, Subject, a label…), how
//! (contains, isn't…), and the value, with every row having to match or any one of them. The rule
//! still keeps its condition in the search language (`mail_client::query`), so the rows are a way
//! of writing that text: [`rows_of`] reads a condition into rows when every part of it has one,
//! and [`query_of`] writes rows back, any-of as `a OR b`. A condition the rows cannot hold
//! (`in:sent`, a word a rule does not know, all-of and any-of mixed) is edited as text.

use crate::ui::copy::fields;

/// What a condition looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::ui) enum Field {
    From,
    To,
    Subject,
    /// Anywhere in the message.
    Text,
    Label,
    Attachment,
    Status,
    Before,
    After,
    /// The message's size: larger or smaller than a size typed as `search` takes it (`5M`).
    Size,
    /// How long ago it came: newer or older than a span (`30d`, `2w`).
    Age,
}

/// Whether the value is to be there or not: for a field that compares, the first comparison it
/// offers (larger, newer) or the second (smaller, older).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::ui) enum Test {
    Is,
    IsNot,
}

/// Whether every row has to match, or any one is enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(in crate::ui) enum Joined {
    #[default]
    All,
    Any,
}

/// One row: `field` `test` `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Condition {
    pub field: Field,
    pub test: Test,
    pub value: String,
}

/// The states [`Field::Status`] picks from, as `is:` spells them, with their words.
pub(in crate::ui) const STATES: [(&str, &str); 6] = [
    ("unread", fields::UNREAD),
    ("read", fields::READ),
    ("starred", fields::STARRED),
    ("unstarred", fields::NOT_STARRED),
    ("pinned", fields::PINNED),
    ("snoozed", fields::SNOOZED),
];

impl Field {
    /// Every field, in the order its menu lists them.
    pub(in crate::ui) const ALL: [Field; 11] = [
        Field::From,
        Field::To,
        Field::Subject,
        Field::Text,
        Field::Label,
        Field::Attachment,
        Field::Status,
        Field::Before,
        Field::After,
        Field::Size,
        Field::Age,
    ];

    pub(in crate::ui) fn name(self) -> &'static str {
        match self {
            Field::From => fields::FROM,
            Field::To => fields::TO,
            Field::Subject => fields::SUBJECT,
            Field::Text => fields::MESSAGE_TEXT,
            Field::Label => fields::LABEL,
            Field::Attachment => fields::ATTACHMENT,
            Field::Status => fields::STATUS,
            Field::Before => fields::RECEIVED_BEFORE,
            Field::After => fields::RECEIVED_AFTER,
            Field::Size => fields::SIZE,
            Field::Age => fields::AGE,
        }
    }

    /// The tests it offers, with their words; a field with one test shows no menu for it.
    pub(in crate::ui) fn tests(self) -> Vec<(Test, &'static str)> {
        match self {
            Field::From | Field::To | Field::Subject | Field::Text => {
                vec![(Test::Is, fields::CONTAINS), (Test::IsNot, fields::LACKS)]
            }
            Field::Label => vec![(Test::Is, fields::IS), (Test::IsNot, fields::IS_NOT)],
            Field::Attachment => vec![(Test::Is, fields::HAS_ONE), (Test::IsNot, fields::HAS_NONE)],
            Field::Size => vec![
                (Test::Is, fields::IS_LARGER),
                (Test::IsNot, fields::IS_SMALLER),
            ],
            Field::Age => vec![
                (Test::Is, fields::IS_NEWER),
                (Test::IsNot, fields::IS_OLDER),
            ],
            Field::Status | Field::Before | Field::After => Vec::new(),
        }
    }

    /// Whether a row of it needs a value.
    pub(in crate::ui) fn takes_value(self) -> bool {
        self != Field::Attachment
    }

    /// A row of it as it starts: the first test, and the first state for Status.
    pub(in crate::ui) fn fresh(self) -> Condition {
        Condition {
            field: self,
            test: Test::Is,
            value: match self {
                Field::Status => STATES[0].0.to_owned(),
                _ => String::new(),
            },
        }
    }

    /// The term's word, for `test`: a comparing field has one for each side.
    fn word_for(self, test: Test) -> &'static str {
        match (self, test) {
            (Field::Size, Test::Is) => "larger",
            (Field::Size, Test::IsNot) => "smaller",
            (Field::Age, Test::Is) => "newer_than",
            (Field::Age, Test::IsNot) => "older_than",
            _ => self.word(),
        }
    }

    /// Whether its two tests are two terms rather than one term and its negation.
    fn compares(self) -> bool {
        matches!(self, Field::Size | Field::Age)
    }

    fn word(self) -> &'static str {
        match self {
            Field::From => "from",
            Field::To => "to",
            Field::Subject => "subject",
            Field::Text => "",
            Field::Label => "label",
            Field::Attachment => "has",
            Field::Status => "is",
            Field::Before => "before",
            Field::After => "after",
            Field::Size => "larger",
            Field::Age => "newer_than",
        }
    }
}

/// `query` as rows and how they join, when every part of it is one a row can hold. Empty is no
/// rows, all of them. Any-of is `a OR b OR c`, a row on each side of every `OR`; a line that mixes
/// `OR` with terms side by side is both at once, which rows cannot show.
pub(in crate::ui) fn rows_of(query: &str) -> Option<(Joined, Vec<Condition>)> {
    let tokens = mail_client::query::tokens(query);
    if !tokens.iter().any(mail_client::query::is_either) {
        return all_of(tokens).map(|rows| (Joined::All, rows));
    }
    // Rows on the even places, `OR` on the odd ones, and a row last.
    if tokens.len().is_multiple_of(2) {
        return None;
    }
    let mut rows = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        let either = mail_client::query::is_either(token);
        if either != (at % 2 == 1) {
            return None;
        }
        if !either {
            rows.push(row_of(token)?);
        }
    }
    Some((Joined::Any, rows))
}

/// Terms side by side, all of which match: words in a row join one row.
fn all_of(tokens: Vec<mail_client::query::Token>) -> Option<Vec<Condition>> {
    let mut rows: Vec<Condition> = Vec::new();
    for token in tokens {
        let row = row_of(&token)?;
        match rows.last_mut() {
            Some(last)
                if row.field == Field::Text
                    && last.field == Field::Text
                    && last.test == row.test
                    && !token.negated =>
            {
                last.value.push(' ');
                last.value.push_str(&row.value);
            }
            _ => rows.push(row),
        }
    }
    Some(rows)
}

/// One token as a row, when a row can hold it.
fn row_of(token: &mail_client::query::Token) -> Option<Condition> {
    let test = if token.negated { Test::IsNot } else { Test::Is };
    let Some((word, value)) = token
        .text
        .split_once(':')
        .filter(|_| !token.quoted || token.text.contains(':'))
    else {
        // A word, or a quoted phrase, anywhere in the message.
        let value = if token.quoted {
            format!("\"{}\"", token.text)
        } else {
            token.text.clone()
        };
        return Some(Condition {
            field: Field::Text,
            test,
            value,
        });
    };
    let (field, test) = match word.to_ascii_lowercase().as_str() {
        "from" => (Field::From, test),
        "to" => (Field::To, test),
        "subject" => (Field::Subject, test),
        "label" => (Field::Label, test),
        "has" if value.eq_ignore_ascii_case("attachment") => (Field::Attachment, test),
        "is" if !token.negated && STATES.iter().any(|(state, _)| *state == value) => {
            (Field::Status, test)
        }
        "before" if !token.negated => (Field::Before, test),
        "after" if !token.negated => (Field::After, test),
        "larger" if !token.negated => (Field::Size, Test::Is),
        "smaller" if !token.negated => (Field::Size, Test::IsNot),
        "newer" | "newer_than" if !token.negated => (Field::Age, Test::Is),
        "older" | "older_than" if !token.negated => (Field::Age, Test::IsNot),
        _ => return None,
    };
    if value.is_empty() {
        return None;
    }
    Some(Condition {
        field,
        test,
        value: if field == Field::Attachment {
            String::new()
        } else {
            value.to_owned()
        },
    })
}

/// `rows` in the search language, joined as `joined` says. A row still without its value is
/// left out: [`complete`] says whether there is one.
pub(in crate::ui) fn query_of(rows: &[Condition], joined: Joined) -> String {
    match joined {
        Joined::All => rows.iter().filter_map(term).collect::<Vec<_>>().join(" "),
        Joined::Any => rows
            .iter()
            .filter_map(one_term)
            .collect::<Vec<_>>()
            .join(" OR "),
    }
}

/// Whether every row has what it needs, and there is at least one.
pub(in crate::ui) fn complete(rows: &[Condition]) -> bool {
    !rows.is_empty()
        && rows
            .iter()
            .all(|row| !row.field.takes_value() || !row.value.trim().is_empty())
}

/// One row in the search language, `None` while it waits for its value.
pub(in crate::ui) fn term(row: &Condition) -> Option<String> {
    let value = row.value.trim().replace('"', "");
    let not = if row.test == Test::IsNot && !row.field.compares() {
        "-"
    } else {
        ""
    };
    match row.field {
        Field::Attachment => Some(format!("{not}has:attachment")),
        _ if value.is_empty() => None,
        Field::Text => {
            let raw = row.value.trim();
            Some(match row.test {
                Test::Is => raw.to_owned(),
                // Each word or phrase kept out on its own.
                Test::IsNot => phrases(raw)
                    .iter()
                    .map(|part| format!("-{part}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            })
        }
        field if value.contains(char::is_whitespace) => {
            Some(format!("{not}{}:\"{value}\"", field.word_for(row.test)))
        }
        field => Some(format!("{not}{}:{value}", field.word_for(row.test))),
    }
}

/// One row as a single term, for a side of an `OR`: a message's words become one phrase.
fn one_term(row: &Condition) -> Option<String> {
    if row.field != Field::Text {
        return term(row);
    }
    let words = phrases(row.value.trim());
    let phrase = match words.as_slice() {
        [] => return None,
        [one] => one.clone(),
        many => format!(
            "\"{}\"",
            many.iter()
                .map(|part| part.trim_matches('"'))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    };
    Some(match row.test {
        Test::Is => phrase,
        Test::IsNot => format!("-{phrase}"),
    })
}

/// `text` split into its words and quoted phrases, each as typed.
fn phrases(text: &str) -> Vec<String> {
    mail_client::query::tokens(text)
        .into_iter()
        .map(|token| {
            if token.quoted {
                format!("\"{}\"", token.text)
            } else {
                token.text
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Condition, Field, Joined, Test, complete, query_of, rows_of};

    fn row(field: Field, test: Test, value: &str) -> Condition {
        Condition {
            field,
            test,
            value: value.to_owned(),
        }
    }

    #[test]
    fn a_condition_reads_into_rows_and_writes_back_as_it_was() {
        let cases: &[(&str, Vec<Condition>)] = &[
            ("", vec![]),
            (
                "from:ada@example.test",
                vec![row(Field::From, Test::Is, "ada@example.test")],
            ),
            (
                "-from:news subject:\"lunch on friday\"",
                vec![
                    row(Field::From, Test::IsNot, "news"),
                    row(Field::Subject, Test::Is, "lunch on friday"),
                ],
            ),
            (
                "invoice due",
                vec![row(Field::Text, Test::Is, "invoice due")],
            ),
            (
                "\"exact words\"",
                vec![row(Field::Text, Test::Is, "\"exact words\"")],
            ),
            ("-spam", vec![row(Field::Text, Test::IsNot, "spam")]),
            (
                "has:attachment -label:work is:unread after:2026-01-01",
                vec![
                    row(Field::Attachment, Test::Is, ""),
                    row(Field::Label, Test::IsNot, "work"),
                    row(Field::Status, Test::Is, "unread"),
                    row(Field::After, Test::Is, "2026-01-01"),
                ],
            ),
        ];
        for (query, rows) in cases {
            assert_eq!(rows_of(query), Some((Joined::All, rows.clone())), "{query}");
            assert_eq!(query_of(rows, Joined::All), *query, "{query}");
        }
    }

    #[test]
    fn a_condition_the_rows_cannot_hold_is_edited_as_text() {
        for query in [
            "in:sent",
            "frm:ada",
            "-is:unread",
            "is:everything",
            "from:",
            // All-of and any-of at once.
            "from:ada subject:lunch OR from:bob",
            "OR from:ada",
            "from:ada OR",
            "-larger:5M",
        ] {
            assert_eq!(rows_of(query), None, "{query}");
        }
    }

    #[test]
    fn a_row_without_its_value_is_left_out_and_holds_save() {
        let rows = vec![
            row(Field::From, Test::Is, "ada"),
            row(Field::Subject, Test::Is, " "),
        ];
        assert_eq!(query_of(&rows, Joined::All), "from:ada");
        assert!(!complete(&rows));
        assert!(!complete(&[]));
        assert!(complete(&[Field::Attachment.fresh()]));
        assert!(complete(&[Field::Status.fresh()]));
    }

    #[test]
    fn words_kept_out_are_kept_out_one_by_one() {
        let rows = vec![row(Field::Text, Test::IsNot, "promo \"last chance\"")];
        assert_eq!(query_of(&rows, Joined::All), "-promo -\"last chance\"");
    }

    #[test]
    fn any_of_reads_into_rows_and_writes_back_as_it_was() {
        let cases: &[(&str, Vec<Condition>)] = &[
            (
                "from:ada OR subject:\"lunch on friday\"",
                vec![
                    row(Field::From, Test::Is, "ada"),
                    row(Field::Subject, Test::Is, "lunch on friday"),
                ],
            ),
            (
                "larger:5M OR older_than:30d OR -label:work",
                vec![
                    row(Field::Size, Test::Is, "5M"),
                    row(Field::Age, Test::IsNot, "30d"),
                    row(Field::Label, Test::IsNot, "work"),
                ],
            ),
            (
                "\"invoice due\" OR -promo",
                vec![
                    row(Field::Text, Test::Is, "\"invoice due\""),
                    row(Field::Text, Test::IsNot, "promo"),
                ],
            ),
        ];
        for (query, rows) in cases {
            assert_eq!(rows_of(query), Some((Joined::Any, rows.clone())), "{query}");
            assert_eq!(query_of(rows, Joined::Any), *query, "{query}");
        }
    }

    #[test]
    fn sizes_and_ages_read_both_ways() {
        let cases: &[(&str, Condition)] = &[
            ("larger:5M", row(Field::Size, Test::Is, "5M")),
            ("smaller:100K", row(Field::Size, Test::IsNot, "100K")),
            ("newer_than:7d", row(Field::Age, Test::Is, "7d")),
            ("older_than:1y", row(Field::Age, Test::IsNot, "1y")),
        ];
        for (query, want) in cases {
            assert_eq!(
                rows_of(query),
                Some((Joined::All, vec![want.clone()])),
                "{query}"
            );
            assert_eq!(query_of(std::slice::from_ref(want), Joined::All), *query);
        }
    }

    /// Words in one row are one phrase on a side of an either-or, so the `OR` keeps its sides.
    #[test]
    fn a_rows_words_are_one_phrase_when_any_will_do() {
        let rows = vec![
            row(Field::Text, Test::Is, "invoice due"),
            row(Field::From, Test::Is, "ada"),
        ];
        assert_eq!(query_of(&rows, Joined::Any), "\"invoice due\" OR from:ada");
    }
}
