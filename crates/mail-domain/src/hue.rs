//! The seven colours a flag can be, and the keywords Apple Mail writes them as.
//!
//! Apple Mail keeps a flag's colour on the server as a three-bit number in three IMAP keywords,
//! `$MailFlagBit0` (the lowest bit) to `$MailFlagBit2`, beside `\Flagged`
//! (draft-eggert-mailflagcolors). No bit set is red, so a message any other client flagged is a
//! red flag here, and a red flag written here is plain `\Flagged` there. The bits mean nothing
//! without `\Flagged`, and unflagging clears them.
//!
//! The same seven are the colours a label can be given: they are Finder's tag colours too.

use serde::{Deserialize, Serialize};

/// One of the seven flag and tag colours, in Apple Mail's menu order.
///
/// `Default` is meaningful, and it is `Red`: a flag with no colour bits is red, every message
/// stored before flags had colours was flagged red if it was flagged at all, and a row written
/// without the field therefore reads back as exactly what it was.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, PartialOrd, Ord, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Hue {
    #[default]
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

/// The keywords that hold a flag's colour, lowest bit first.
pub const FLAG_BIT_KEYWORDS: [&str; 3] = ["$MailFlagBit0", "$MailFlagBit1", "$MailFlagBit2"];

impl Hue {
    /// Every hue, in menu order.
    pub const ALL: [Hue; 7] = [
        Hue::Red,
        Hue::Orange,
        Hue::Yellow,
        Hue::Green,
        Hue::Blue,
        Hue::Purple,
        Hue::Gray,
    ];

    /// The hue's word: the name serde writes, and the one a label's colour is stored as.
    pub const fn as_str(self) -> &'static str {
        match self {
            Hue::Red => "red",
            Hue::Orange => "orange",
            Hue::Yellow => "yellow",
            Hue::Green => "green",
            Hue::Blue => "blue",
            Hue::Purple => "purple",
            Hue::Gray => "gray",
        }
    }

    /// The hue a stored word names, or `None` for any other text: a label coloured `#rrggbb` by
    /// an earlier build, or nothing at all.
    pub fn parse(word: &str) -> Option<Hue> {
        Hue::ALL.into_iter().find(|hue| hue.as_str() == word)
    }

    /// Apple Mail's number for this colour: bit 0 is `$MailFlagBit0`.
    pub const fn flag_bits(self) -> u8 {
        match self {
            Hue::Red => 0b000,
            Hue::Orange => 0b001,
            Hue::Yellow => 0b010,
            Hue::Green => 0b011,
            Hue::Blue => 0b100,
            Hue::Purple => 0b101,
            Hue::Gray => 0b110,
        }
    }

    /// The colour Apple Mail's number names. `0b111` is no colour Apple Mail writes, and is read
    /// as red, the colour of a flag that says none.
    pub const fn from_flag_bits(bits: u8) -> Hue {
        match bits & 0b111 {
            0b001 => Hue::Orange,
            0b010 => Hue::Yellow,
            0b011 => Hue::Green,
            0b100 => Hue::Blue,
            0b101 => Hue::Purple,
            0b110 => Hue::Gray,
            _ => Hue::Red,
        }
    }

    /// The colour a message's keywords give its flag. Keywords are compared without regard to
    /// case, as IMAP compares them, and every keyword but the three is ignored.
    pub fn from_keywords<S: AsRef<str>>(keywords: &[S]) -> Hue {
        let bits = FLAG_BIT_KEYWORDS
            .iter()
            .enumerate()
            .filter(|(_, bit)| {
                keywords
                    .iter()
                    .any(|k| k.as_ref().eq_ignore_ascii_case(bit))
            })
            .fold(0u8, |bits, (at, _)| bits | (1 << at));
        Hue::from_flag_bits(bits)
    }

    /// The colour keywords to set and to clear for this colour: `(set, clear)`, together always
    /// the three.
    pub fn flag_keywords(self) -> (Vec<&'static str>, Vec<&'static str>) {
        let bits = self.flag_bits();
        let mut set = Vec::new();
        let mut clear = Vec::new();
        for (at, keyword) in FLAG_BIT_KEYWORDS.iter().enumerate() {
            if bits & (1 << at) != 0 {
                set.push(*keyword);
            } else {
                clear.push(*keyword);
            }
        }
        (set, clear)
    }
}

impl std::fmt::Display for Hue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(hue, bit0 bit1 bit2 as Apple Mail's table writes them)`.
    const TABLE: [(Hue, &str); 7] = [
        (Hue::Red, "000"),
        (Hue::Orange, "100"),
        (Hue::Yellow, "010"),
        (Hue::Green, "110"),
        (Hue::Blue, "001"),
        (Hue::Purple, "101"),
        (Hue::Gray, "011"),
    ];

    fn bits_of(table: &str) -> u8 {
        table
            .chars()
            .enumerate()
            .filter(|(_, c)| *c == '1')
            .fold(0, |bits, (at, _)| bits | (1 << at))
    }

    #[test]
    fn each_hue_is_the_bits_apple_mail_gives_it_and_back() {
        for (hue, table) in TABLE {
            assert_eq!(hue.flag_bits(), bits_of(table), "{hue} as bits");
            assert_eq!(Hue::from_flag_bits(bits_of(table)), hue, "{table} as a hue");
        }
    }

    #[test]
    fn keywords_read_as_the_hue_they_spell_and_a_hue_writes_them() {
        for (hue, table) in TABLE {
            let (set, clear) = hue.flag_keywords();
            let want: Vec<&str> = FLAG_BIT_KEYWORDS
                .iter()
                .zip(table.chars())
                .filter(|(_, c)| *c == '1')
                .map(|(k, _)| *k)
                .collect();
            assert_eq!(set, want, "{hue} sets");
            assert_eq!(set.len() + clear.len(), 3, "{hue} names every bit once");
            assert!(
                clear.iter().all(|k| !set.contains(k)),
                "{hue} clears what it sets"
            );
            let mut read = set.clone();
            read.extend(["$MDNSent", "\\Seen", "$label1"]);
            assert_eq!(Hue::from_keywords(&read), hue, "{hue} read back");
        }
    }

    #[test]
    fn keywords_are_read_without_regard_to_case_and_nothing_is_red() {
        assert_eq!(Hue::from_keywords(&["$mailflagbit2"]), Hue::Blue);
        assert_eq!(Hue::from_keywords::<&str>(&[]), Hue::Red);
        assert_eq!(Hue::from_flag_bits(0b111), Hue::Red);
    }

    #[test]
    fn a_hue_is_stored_as_its_word() {
        for hue in Hue::ALL {
            assert_eq!(Hue::parse(hue.as_str()), Some(hue));
            assert_eq!(
                serde_json::to_string(&hue).expect("serialize"),
                format!("\"{hue}\"")
            );
        }
        assert_eq!(Hue::parse("#ff0000"), None);
    }
}
