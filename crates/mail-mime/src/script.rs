//! Which CJK script a message is written in, from what the message itself says.
//!
//! Chinese, Japanese and Korean share the Han ideographs, but not their shapes: the same code
//! point is drawn one way in Taiwan, another in mainland China, another in Japan. A renderer that
//! has to choose a face for a run of ideographs needs to know which of the four the writer meant,
//! and nothing in the code points alone says so for sure. This module reads the three places a
//! message says it, most explicit first:
//!
//! 1. `Content-Language` (RFC 3282), when it names a CJK language specifically enough:
//!    `zh-TW`, `zh-Hant`, `ja`, `ko`;
//! 2. the `charset` the body was sent in, when it is one only one of the four uses: Big5 is
//!    Traditional Chinese, GBK Simplified, Shift_JIS Japanese, EUC-KR Korean;
//! 3. the text: kana is Japanese, Hangul Korean, and ideographs alone are Chinese, Traditional or
//!    Simplified by which of the characters the two forms spell differently it uses more.
//!
//! `None` means the message does not say, or is not CJK at all: the caller's own default (a
//! locale's, say) is then as good a guess as any. Pure: text in, a hint out.

use crate::parse::Parsed;

/// A CJK writing system, as far as choosing a face for it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Script {
    /// Chinese in traditional characters: Taiwan, Hong Kong, Macau.
    TraditionalChinese,
    /// Chinese in simplified characters: mainland China, Singapore.
    SimplifiedChinese,
    Japanese,
    Korean,
}

impl Script {
    /// The BCP 47 tag for the script: `zh-Hant`, `zh-Hans`, `ja`, `ko`.
    pub fn tag(self) -> &'static str {
        match self {
            Script::TraditionalChinese => "zh-Hant",
            Script::SimplifiedChinese => "zh-Hans",
            Script::Japanese => "ja",
            Script::Korean => "ko",
        }
    }

    /// The script a BCP 47 language tag names, when it names one specifically: `zh-TW`,
    /// `zh-Hant-HK`, `ja-JP`, `ko`. A bare `zh` does not say which Chinese, so is `None`, as is
    /// every tag that is not Chinese, Japanese or Korean. Case does not matter.
    pub fn from_tag(tag: &str) -> Option<Script> {
        let lower = tag.trim().to_ascii_lowercase();
        let mut subtags = lower.split(['-', '_']);
        match subtags.next()? {
            "ja" | "jpn" => Some(Script::Japanese),
            "ko" | "kor" => Some(Script::Korean),
            "zh" | "zho" | "chi" | "cmn" | "yue" => {
                let mut script = None;
                for subtag in subtags {
                    script = script.or(match subtag {
                        "hant" | "tw" | "hk" | "mo" => Some(Script::TraditionalChinese),
                        "hans" | "cn" | "sg" | "my" => Some(Script::SimplifiedChinese),
                        _ => None,
                    });
                }
                script
            }
            _ => None,
        }
    }

    /// The script a `Content-Language` value names: the first of its tags that names one.
    pub fn from_content_language(value: &str) -> Option<Script> {
        value.split(',').find_map(Script::from_tag)
    }

    /// The script a MIME `charset` is used for, when only one of the four uses it. UTF-8, and
    /// every charset that is not CJK, says nothing.
    pub fn from_charset(charset: &str) -> Option<Script> {
        let lower = charset.trim().trim_matches('"').to_ascii_lowercase();
        match lower.as_str() {
            "big5" | "big5-hkscs" | "cp950" | "x-euc-tw" | "euc-tw" => {
                Some(Script::TraditionalChinese)
            }
            "gb2312" | "gbk" | "gb18030" | "hz-gb-2312" | "euc-cn" | "cp936" | "x-gbk"
            | "csgb2312" => Some(Script::SimplifiedChinese),
            "shift_jis" | "shift-jis" | "sjis" | "x-sjis" | "windows-31j" | "cp932" | "euc-jp"
            | "x-euc-jp" | "iso-2022-jp" | "csiso2022jp" => Some(Script::Japanese),
            "euc-kr" | "ks_c_5601-1987" | "cp949" | "windows-949" | "iso-2022-kr"
            | "x-windows-949" => Some(Script::Korean),
            _ => None,
        }
    }

    /// The script `text` is written in, by its characters. See the module notes.
    pub fn from_text(text: &str) -> Option<Script> {
        let mut count = Count::default();
        for ch in text.chars().take(TEXT_BUDGET) {
            count.add(ch);
        }
        count.verdict()
    }
}

/// How much of a text [`Script::from_text`] reads: the first page or so says as much as the
/// whole of a long thread would.
const TEXT_BUDGET: usize = 20_000;

/// Characters written one way in Traditional Chinese and another in Simplified, among the most
/// common in either: each position of [`TRADITIONAL`] is the same character as that position of
/// [`SIMPLIFIED`]. Only pairs where neither form is also an ordinary character of the other
/// system are listed (`後`/`后` is not: `后` is a Traditional character in its own right).
const TRADITIONAL: &str = "這們個說國時會來對為過還發開關長問間學經義現當點樣讓應與從見無動實業請謝議錄報紀聽話認識題頭東車門書電腦買賣錢飛氣愛邊視體進號處級夠寫語歡費區單總麗連運鐘";
const SIMPLIFIED: &str = "这们个说国时会来对为过还发开关长问间学经义现当点样让应与从见无动实业请谢议录报纪听话认识题头东车门书电脑买卖钱飞气爱边视体进号处级够写语欢费区单总丽连运钟";

/// What [`Script::from_text`] has seen.
#[derive(Default)]
struct Count {
    kana: usize,
    hangul: usize,
    han: usize,
    traditional: usize,
    simplified: usize,
}

impl Count {
    fn add(&mut self, ch: char) {
        match ch {
            '\u{3040}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' | '\u{ff66}'..='\u{ff9f}' => {
                // The prolonged sound mark and the middle dot sit in the katakana block but are
                // used in Chinese text too; they say nothing on their own.
                if !matches!(ch, '\u{30fb}' | '\u{30fc}') {
                    self.kana += 1;
                }
            }
            '\u{ac00}'..='\u{d7af}' | '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}' => {
                self.hangul += 1;
            }
            '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' | '\u{f900}'..='\u{faff}' => {
                self.han += 1;
                if TRADITIONAL.contains(ch) {
                    self.traditional += 1;
                }
                if SIMPLIFIED.contains(ch) {
                    self.simplified += 1;
                }
            }
            _ => {}
        }
    }

    fn verdict(&self) -> Option<Script> {
        let cjk = self.kana + self.hangul + self.han;
        // Japanese prose is a quarter kana or more; a stray `の` in a Chinese sentence is not.
        let japanese = self.kana > 0 && self.kana * 4 >= cjk;
        // Korean is written almost wholly in Hangul, with the odd ideograph.
        let korean = self.hangul > 0 && self.hangul * 2 >= cjk;
        match (japanese, korean) {
            (true, true) if self.hangul > self.kana => Some(Script::Korean),
            (true, _) => Some(Script::Japanese),
            (false, true) => Some(Script::Korean),
            (false, false) if self.traditional > self.simplified => {
                Some(Script::TraditionalChinese)
            }
            (false, false) if self.simplified > self.traditional => Some(Script::SimplifiedChinese),
            (false, false) => None,
        }
    }
}

/// The script a message is written in: its `Content-Language`, else its body's `charset`, else
/// its `text` (the caller's choice: the subject and the body, say). `None` when none of them
/// says. See the module notes.
pub fn script_of(
    content_language: Option<&str>,
    charset: Option<&str>,
    text: &str,
) -> Option<Script> {
    content_language
        .and_then(Script::from_content_language)
        .or_else(|| charset.and_then(Script::from_charset))
        .or_else(|| Script::from_text(text))
}

impl Parsed {
    /// The script this message is written in, by its own `Content-Language` and `charset` and
    /// then by `subject` and its body's text. See [`script_of`].
    pub fn script(&self, subject: &str) -> Option<Script> {
        let mut text = String::from(subject);
        text.push('\n');
        text.push_str(self.html.as_deref().or(self.text.as_deref()).unwrap_or(""));
        script_of(
            self.content_language.as_deref(),
            self.charset.as_deref(),
            &text,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_spellings_pair_up_and_never_overlap() {
        let traditional: Vec<char> = TRADITIONAL.chars().collect();
        let simplified: Vec<char> = SIMPLIFIED.chars().collect();
        assert_eq!(traditional.len(), simplified.len());
        for (t, s) in traditional.iter().zip(&simplified) {
            assert_ne!(t, s);
            assert!(!SIMPLIFIED.contains(*t), "{t} is in both lists");
            assert!(!TRADITIONAL.contains(*s), "{s} is in both lists");
        }
    }

    #[test]
    fn a_tag_names_a_script_only_when_it_is_specific() {
        let cases = [
            ("ja", Some(Script::Japanese)),
            ("ja-JP", Some(Script::Japanese)),
            ("KO-kr", Some(Script::Korean)),
            ("zh-TW", Some(Script::TraditionalChinese)),
            ("zh-Hant-HK", Some(Script::TraditionalChinese)),
            ("zh_HK", Some(Script::TraditionalChinese)),
            ("zh-CN", Some(Script::SimplifiedChinese)),
            ("zh-Hans", Some(Script::SimplifiedChinese)),
            ("zh", None),
            ("en-US", None),
            ("", None),
        ];
        for (tag, want) in cases {
            assert_eq!(Script::from_tag(tag), want, "{tag}");
        }
        assert_eq!(
            Script::from_content_language("en, ja"),
            Some(Script::Japanese)
        );
    }

    #[test]
    fn a_charset_names_a_script_only_when_one_uses_it() {
        assert_eq!(
            Script::from_charset("Big5"),
            Some(Script::TraditionalChinese)
        );
        assert_eq!(
            Script::from_charset("\"GB2312\""),
            Some(Script::SimplifiedChinese)
        );
        assert_eq!(Script::from_charset("ISO-2022-JP"), Some(Script::Japanese));
        assert_eq!(Script::from_charset("ks_c_5601-1987"), Some(Script::Korean));
        assert_eq!(Script::from_charset("utf-8"), None);
        assert_eq!(Script::from_charset("iso-8859-1"), None);
    }

    #[test]
    fn the_text_says_what_the_headers_do_not() {
        let cases = [
            ("會議紀錄已經寄出", Some(Script::TraditionalChinese)),
            ("会议纪录已经寄出", Some(Script::SimplifiedChinese)),
            ("会議の記録を送りました", Some(Script::Japanese)),
            ("了解です", Some(Script::Japanese)),
            ("회의록을 보냈습니다", Some(Script::Korean)),
            ("韓國語 한국어 문장입니다", Some(Script::Korean)),
            // Ideographs both forms share say nothing about which.
            ("中文", None),
            // One `の` in a Chinese sentence does not make it Japanese.
            (
                "我們今天的會議記錄已經整理好了の",
                Some(Script::TraditionalChinese),
            ),
            ("The kestrel report", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(Script::from_text(text), want, "{text}");
        }
    }

    #[test]
    fn the_headers_come_before_the_text() {
        // Kana in the text, but the message says it is Korean.
        assert_eq!(
            script_of(Some("ko"), None, "ありがとう"),
            Some(Script::Korean)
        );
        // A bare `zh` defers to the charset, and the charset to the text.
        assert_eq!(
            script_of(Some("zh"), Some("gbk"), "會議"),
            Some(Script::SimplifiedChinese)
        );
        assert_eq!(
            script_of(Some("zh"), Some("utf-8"), "會議"),
            Some(Script::TraditionalChinese)
        );
    }
}
