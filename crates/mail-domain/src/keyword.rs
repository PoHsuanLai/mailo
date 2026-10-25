//! Labels as IMAP keywords, on a server with no labels of its own
//! ([`crate::ServerLabels::Keywords`]).
//!
//! A keyword is an atom: ASCII, no space, none of `( ) { % * " \ ]`. A label's name becomes one
//! by keeping its letters, digits, `-` and `.`, and writing `_` for every run of anything else.
//! A name with no letter or digit in it has no keyword, and its label stays on this computer.
//!
//! Thunderbird's five tags are keywords `$label1` to `$label5`, and are read and written as
//! labels with Thunderbird's names and colours, so a tag set in one is a label in the other.

use crate::hue::Hue;

/// Thunderbird's built-in tags: the keyword, the name it shows and the colour it draws.
pub const THUNDERBIRD_TAGS: [(&str, &str, Hue); 5] = [
    ("$label1", "Important", Hue::Red),
    ("$label2", "Work", Hue::Orange),
    ("$label3", "Personal", Hue::Green),
    ("$label4", "To Do", Hue::Blue),
    ("$label5", "Later", Hue::Purple),
];

/// Keywords with no `$` that other clients already give a meaning other than a label: junk
/// verdicts. A label spelled as one is kept local, and one found on a message is not a label.
const RESERVED: [&str; 5] = ["Junk", "NonJunk", "NotJunk", "JunkRecorded", "Forwarded"];

/// The keyword a label called `name` is written to the server as, or `None` when it has none.
pub fn label_keyword(name: &str) -> Option<String> {
    let name = name.trim();
    if let Some((keyword, _, _)) = THUNDERBIRD_TAGS
        .iter()
        .find(|(_, tag, _)| tag.eq_ignore_ascii_case(name))
    {
        return Some((*keyword).to_owned());
    }
    let mut atom = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
            atom.push(c);
        } else if !atom.ends_with('_') {
            atom.push('_');
        }
    }
    let atom = atom.trim_matches('_');
    let has_word = atom.chars().any(|c| c.is_ascii_alphanumeric());
    let reserved = RESERVED.iter().any(|r| r.eq_ignore_ascii_case(atom));
    (has_word && !reserved).then(|| atom.to_owned())
}

/// The label a keyword on a message stands for: its name, and its colour where the keyword is
/// one of Thunderbird's tags. `None` for a keyword that is not a label: a registered `$` keyword
/// (`$MDNSent`, `$Junk`, the flag colour bits), a system flag, or a reserved word.
pub fn keyword_label(keyword: &str) -> Option<(String, Option<Hue>)> {
    if let Some((_, name, hue)) = THUNDERBIRD_TAGS
        .iter()
        .find(|(tag, _, _)| tag.eq_ignore_ascii_case(keyword))
    {
        return Some(((*name).to_owned(), Some(*hue)));
    }
    if keyword.starts_with(['$', '\\']) {
        return None;
    }
    // Only a keyword this client could have written: anything else is some other client's
    // vocabulary, which a label of the same spelling would turn into a second meaning.
    (label_keyword(keyword).as_deref() == Some(keyword)).then(|| (keyword.to_owned(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_an_atom_or_nothing() {
        let cases = [
            ("Receipts", Some("Receipts")),
            ("Project X", Some("Project_X")),
            ("  a  (b)  ", Some("a_b")),
            ("v1.2-rc", Some("v1.2-rc")),
            ("100%", Some("100")),
            ("\\Seen", Some("Seen")),
            ("$Junk", None),
            ("Café", Some("Caf")),
            ("工作", None),
            ("***", None),
            ("", None),
            ("junk", None),
        ];
        for (name, want) in cases {
            assert_eq!(label_keyword(name).as_deref(), want, "{name:?}");
        }
    }

    #[test]
    fn thunderbirds_tags_are_its_keywords_both_ways() {
        for (keyword, name, hue) in THUNDERBIRD_TAGS {
            assert_eq!(label_keyword(name).as_deref(), Some(keyword), "{name}");
            assert_eq!(
                label_keyword(&name.to_uppercase()).as_deref(),
                Some(keyword),
                "{name} in capitals"
            );
            assert_eq!(
                keyword_label(keyword),
                Some((name.to_owned(), Some(hue))),
                "{keyword}"
            );
        }
        assert_eq!(
            keyword_label("$LABEL4"),
            Some(("To Do".to_owned(), Some(Hue::Blue)))
        );
    }

    #[test]
    fn only_a_keyword_this_client_could_write_is_a_label() {
        let cases = [
            ("Receipts", Some("Receipts")),
            ("Project_X", Some("Project_X")),
            ("$MDNSent", None),
            ("$MailFlagBit0", None),
            ("$Junk", None),
            ("NonJunk", None),
            ("\\Flagged", None),
            ("_odd_", None),
        ];
        for (keyword, want) in cases {
            assert_eq!(
                keyword_label(keyword).map(|(name, _)| name).as_deref(),
                want,
                "{keyword}"
            );
        }
    }

    #[test]
    fn a_label_read_back_from_its_keyword_writes_the_same_keyword() {
        for name in ["Project X", "Receipts", "Work", "a (b)"] {
            let keyword = label_keyword(name).expect("a keyword");
            let (read, _) = keyword_label(&keyword).expect("a label");
            assert_eq!(label_keyword(&read), Some(keyword), "{name}");
        }
    }
}
