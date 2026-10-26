//! A rule's condition as rows built from menus: what is looked at (From, Subject, a label…), how
//! (contains, isn't…), and the value. The rule still keeps its condition in the search language
//! (`mail_client::query`), so the rows are a way of writing that text: [`rows_of`] reads a condition
//! into rows when every part of it has one, and [`query_of`] writes rows back. A condition the
//! rows cannot hold (`in:sent`, a word a rule does not know) is edited as text.

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
}

/// Whether the value is to be there or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::ui) enum Test {
    Is,
    IsNot,
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
    pub(in crate::ui) const ALL: [Field; 9] = [
        Field::From,
        Field::To,
        Field::Subject,
        Field::Text,
        Field::Label,
        Field::Attachment,
        Field::Status,
        Field::Before,
        Field::After,
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
        }
    }
}

/// `query` as rows, when every part of it is one a row can hold. Empty is no rows.
pub(in crate::ui) fn rows_of(query: &str) -> Option<Vec<Condition>> {
    let mut rows: Vec<Condition> = Vec::new();
    for token in mail_client::query::tokens(query) {
        let test = if token.negated { Test::IsNot } else { Test::Is };
        let Some((word, value)) = token
            .text
            .split_once(':')
            .filter(|_| !token.quoted || token.text.contains(':'))
        else {
            // A word, or a quoted phrase, anywhere in the message. Words in a row join one row.
            let value = if token.quoted {
                format!("\"{}\"", token.text)
            } else {
                token.text.clone()
            };
            match rows.last_mut() {
                Some(last) if last.field == Field::Text && last.test == test && !token.negated => {
                    last.value.push(' ');
                    last.value.push_str(&value);
                }
                _ => rows.push(Condition {
                    field: Field::Text,
                    test,
                    value,
                }),
            }
            continue;
        };
        let field = match word.to_ascii_lowercase().as_str() {
            "from" => Field::From,
            "to" => Field::To,
            "subject" => Field::Subject,
            "label" => Field::Label,
            "has" if value.eq_ignore_ascii_case("attachment") => Field::Attachment,
            "is" if !token.negated && STATES.iter().any(|(state, _)| *state == value) => {
                Field::Status
            }
            "before" if !token.negated => Field::Before,
            "after" if !token.negated => Field::After,
            _ => return None,
        };
        if value.is_empty() {
            return None;
        }
        rows.push(Condition {
            field,
            test,
            value: if field == Field::Attachment {
                String::new()
            } else {
                value.to_owned()
            },
        });
    }
    Some(rows)
}

/// `rows` in the search language, every row having to match. A row still without its value is
/// left out: [`complete`] says whether there is one.
pub(in crate::ui) fn query_of(rows: &[Condition]) -> String {
    rows.iter().filter_map(term).collect::<Vec<_>>().join(" ")
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
    let not = if row.test == Test::IsNot { "-" } else { "" };
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
            Some(format!("{not}{}:\"{value}\"", field.word()))
        }
        field => Some(format!("{not}{}:{value}", field.word())),
    }
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
    use super::{Condition, Field, Test, complete, query_of, rows_of};

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
            assert_eq!(rows_of(query).as_ref(), Some(rows), "{query}");
            assert_eq!(query_of(rows), *query, "{query}");
        }
    }

    #[test]
    fn a_condition_the_rows_cannot_hold_is_edited_as_text() {
        for query in ["in:sent", "frm:ada", "-is:unread", "is:everything", "from:"] {
            assert_eq!(rows_of(query), None, "{query}");
        }
    }

    #[test]
    fn a_row_without_its_value_is_left_out_and_holds_save() {
        let rows = vec![
            row(Field::From, Test::Is, "ada"),
            row(Field::Subject, Test::Is, " "),
        ];
        assert_eq!(query_of(&rows), "from:ada");
        assert!(!complete(&rows));
        assert!(!complete(&[]));
        assert!(complete(&[Field::Attachment.fresh()]));
        assert!(complete(&[Field::Status.fresh()]));
    }

    #[test]
    fn words_kept_out_are_kept_out_one_by_one() {
        let rows = vec![row(Field::Text, Test::IsNot, "promo \"last chance\"")];
        assert_eq!(query_of(&rows), "-promo -\"last chance\"");
    }
}
