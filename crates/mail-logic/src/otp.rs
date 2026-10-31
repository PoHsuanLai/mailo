//! One-time codes: the verification or sign-in code a message carries, found so the reader
//! can offer it for copying.
//!
//! [`code`] is a pure function of a subject and a plain body, and a guess, so it guesses
//! narrowly. A code is four to eight digits standing alone, or a short upper-case group joined
//! by a hyphen (`G-123456`, `ABC-DEF`), and it counts only near a word that says what it is —
//! "code", "verification", "one-time", "sign-in", 验证码, 認証コード, Bestätigungscode, código —
//! after it by a line or so, or just before it ("123456 is your code"). Whatever else a message
//! numbers is ruled out by what stands around it: a year, a price beside its currency, a date or
//! a time or a telephone number whose digits run on past a separator, a number labelled as an
//! order, an account or a ticket, and anything inside a link. A promo code, a postcode and an
//! error code are codes of another kind, and the word "code" beside them is not taken as a
//! keyword.
//!
//! Reading a stored message's text for it is `mail-core`'s (`message::looks`).

/// How much of a body is read for a code. One sits near the top of the message it is for, and a
/// long newsletter is not read to the end for a number.
const READ: usize = 4000;

/// How far after its keyword a code may start, in characters: the rest of the sentence, a
/// colon and a blank line.
const AFTER: usize = 80;

/// How far before its keyword a code may end: "482913 is your verification code".
const BEFORE: usize = 40;

/// The code in a message with this `subject` and plain `body`, if it carries one.
pub fn code(subject: &str, body: &str) -> Option<String> {
    let mut text: String = subject.to_owned();
    text.push('\n');
    text.extend(body.chars().take(READ));
    let text = Text::new(&text);
    let keywords = text.keywords();
    let candidates: Vec<Span> = text
        .candidates()
        .into_iter()
        .filter(|c| !keywords.iter().any(|k| c.start < k.end && k.start < c.end))
        .collect();
    keywords.iter().find_map(|keyword| {
        let after = candidates
            .iter()
            .find(|c| c.start >= keyword.end && c.start - keyword.end <= AFTER);
        let before = || {
            candidates
                .iter()
                .rev()
                .find(|c| c.end <= keyword.start && keyword.start - c.end <= BEFORE)
        };
        after.or_else(before).map(|c| text.slice(c))
    })
}

/// A stretch of the text, in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

/// How a keyword must sit among the letters around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bound {
    /// A word of its own at the start, so "code" is not found in "barcode". Written in a script
    /// without spaces, it is found wherever it is.
    Start,
    /// A word of its own at both ends but for a plural `s`, so "code" is not found in "codec"
    /// nor "otp" in "otpauth".
    Both,
    /// Anywhere: Chinese and Japanese are written without spaces between words.
    Anywhere,
}

/// The words a code is found near, in lower case, and how each must sit.
const KEYWORDS: [(&str, Bound); 25] = [
    ("code", Bound::Both),
    ("codes", Bound::Both),
    ("verification", Bound::Start),
    ("verify", Bound::Start),
    ("one-time", Bound::Start),
    ("one time", Bound::Start),
    ("otp", Bound::Both),
    ("passcode", Bound::Start),
    ("sign-in", Bound::Start),
    ("sign in", Bound::Start),
    ("login", Bound::Both),
    ("log in", Bound::Start),
    ("bestätigungscode", Bound::Start),
    ("código", Bound::Start),
    ("codigo", Bound::Start),
    ("驗證碼", Bound::Anywhere),
    ("验证码", Bound::Anywhere),
    ("認證碼", Bound::Anywhere),
    ("认证码", Bound::Anywhere),
    ("動態密碼", Bound::Anywhere),
    ("动态密码", Bound::Anywhere),
    ("認証コード", Bound::Anywhere),
    ("確認コード", Bound::Anywhere),
    ("驗證代碼", Bound::Anywhere),
    ("ワンタイムパスワード", Bound::Anywhere),
];

/// The words that make "code" a code of another kind when they come just before it: a promo
/// code, a postcode, an error code.
const NOT_BEFORE_CODE: [&str; 28] = [
    "promo",
    "promotional",
    "discount",
    "coupon",
    "gift",
    "voucher",
    "referral",
    "zip",
    "postal",
    "post",
    "area",
    "country",
    "error",
    "status",
    "source",
    "qr",
    "bar",
    "tracking",
    "reference",
    "booking",
    "dress",
    "invite",
    "invitation",
    "product",
    "tax",
    "swift",
    "sort",
    "colour",
];

/// The words that make "code" (or "código") a code of another kind when they come just after
/// it: un code promo, un código postal, a code review.
const NOT_AFTER_CODE: [&str; 11] = [
    "postal",
    "promo",
    "promocional",
    "promotionnel",
    "descuento",
    "réduction",
    "reducción",
    "review",
    "reviews",
    "de descuento",
    "de réduction",
];

/// Labels that say a number is something other than a code, among the two words before it.
const LABELS: [&str; 30] = [
    "order",
    "invoice",
    "ticket",
    "tracking",
    "account",
    "acct",
    "reference",
    "ref",
    "case",
    "receipt",
    "phone",
    "tel",
    "telephone",
    "mobile",
    "call",
    "fax",
    "booking",
    "member",
    "customer",
    "transaction",
    "id",
    "pr",
    "issue",
    "pedido",
    "commande",
    "bestellung",
    "rechnung",
    "factura",
    "facture",
    "konto",
];

/// The same in scripts without spaces, found in the few characters before the number.
const CJK_LABELS: [&str; 10] = [
    "订单", "訂單", "单号", "單號", "电话", "電話", "手机", "手機", "注文", "番号",
];

/// What marks a price on either side of its number.
const CURRENCY: [char; 11] = ['$', '€', '£', '¥', '￥', '₹', '₩', '元', '円', '%', '₽'];

/// The text, one character at a time, as written and folded to lower case at the same places.
struct Text {
    chars: Vec<char>,
    lower: Vec<char>,
}

impl Text {
    /// `text`, with full-width digits, letters and hyphens (`１２３４`) read as the ASCII they
    /// stand for and every kind of space as a space.
    fn new(text: &str) -> Text {
        let chars: Vec<char> = text.chars().map(narrow).collect();
        // One character for one, so a place in one is the same place in the other.
        let lower = chars
            .iter()
            .map(|c| c.to_lowercase().next().unwrap_or(*c))
            .collect();
        Text { chars, lower }
    }

    fn slice(&self, span: &Span) -> String {
        self.chars[span.start..span.end].iter().collect()
    }

    fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).copied()
    }

    /// The character before `i`, if there is one.
    fn before(&self, i: usize) -> Option<char> {
        i.checked_sub(1).and_then(|i| self.at(i))
    }

    /// Whether the lower-cased text at `i` reads `word`.
    fn reads(&self, i: usize, word: &[char]) -> bool {
        self.lower.get(i..i + word.len()) == Some(word)
    }

    /// Every keyword in the text, in order, but the ones that name a code of another kind.
    fn keywords(&self) -> Vec<Span> {
        let words: Vec<(Vec<char>, Bound)> = KEYWORDS
            .iter()
            .map(|(word, bound)| (word.chars().collect(), *bound))
            .collect();
        let mut found: Vec<Span> = Vec::new();
        for i in 0..self.lower.len() {
            for (word, bound) in &words {
                if !self.reads(i, word) {
                    continue;
                }
                let end = i + word.len();
                let letter = |at: usize| self.at(at).is_some_and(char::is_alphabetic);
                let starts = !self.before(i).is_some_and(char::is_alphabetic);
                let ends = !letter(end) || (self.at(end) == Some('s') && !letter(end + 1));
                let sits = match bound {
                    Bound::Anywhere => true,
                    Bound::Start => starts,
                    Bound::Both => starts && ends,
                };
                if sits && !self.other_code(i, end, word) {
                    found.push(Span { start: i, end });
                }
            }
        }
        found.sort_by_key(|span| span.start);
        found
    }

    /// Whether the "code" at `start..end` is a code of another kind: a promo code, a code
    /// postal.
    fn other_code(&self, start: usize, end: usize, word: &[char]) -> bool {
        let is_code = |w: &str| word.iter().copied().eq(w.chars());
        if !(is_code("code") || is_code("codes") || is_code("código") || is_code("codigo")) {
            return false;
        }
        let before = self.words_before(start, 1);
        let after = self.words_after(end, 2);
        before.iter().any(|w| NOT_BEFORE_CODE.contains(&w.as_str()))
            || after
                .first()
                .is_some_and(|w| NOT_AFTER_CODE.contains(&w.as_str()))
            || NOT_AFTER_CODE.contains(&after.join(" ").as_str())
    }

    /// Up to `n` words before `i`, nearest first, in lower case. A word here is a run of
    /// letters; what lies between them (spaces, a colon, a `#`) is passed over.
    fn words_before(&self, i: usize, n: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut at = i;
        while out.len() < n {
            while at > 0 && !self.lower[at - 1].is_alphanumeric() {
                if self.lower[at - 1] == '\n' {
                    return out;
                }
                at -= 1;
            }
            let end = at;
            while at > 0 && self.lower[at - 1].is_alphanumeric() {
                at -= 1;
            }
            if at == end {
                break;
            }
            out.push(self.lower[at..end].iter().collect());
        }
        out
    }

    /// Up to `n` words after `i`, in order, in lower case.
    fn words_after(&self, i: usize, n: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut at = i;
        let len = self.lower.len();
        while out.len() < n {
            while at < len && !self.lower[at].is_alphanumeric() {
                if self.lower[at] == '\n' {
                    return out;
                }
                at += 1;
            }
            let start = at;
            while at < len && self.lower[at].is_alphanumeric() {
                at += 1;
            }
            if at == start {
                break;
            }
            out.push(self.lower[start..at].iter().collect());
        }
        out
    }

    /// Every stretch that could be a code, by its shape and what stands around it, in order.
    fn candidates(&self) -> Vec<Span> {
        let mut out = Vec::new();
        let len = self.chars.len();
        let mut i = 0;
        while i < len {
            if !self.chars[i].is_ascii_alphanumeric() {
                i += 1;
                continue;
            }
            let start = i;
            while i < len && self.chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let first = Span { start, end: i };
            // A group joined by a hyphen: `G-123456`.
            if self.at(i) == Some('-') && self.at(i + 1).is_some_and(|c| c.is_ascii_alphanumeric())
            {
                let mut j = i + 1;
                while j < len && self.chars[j].is_ascii_alphanumeric() {
                    j += 1;
                }
                let second = Span {
                    start: i + 1,
                    end: j,
                };
                if self.grouped(first, second) {
                    out.push(Span { start, end: j });
                }
                i = j;
                continue;
            }
            if self.digits(first) {
                out.push(first);
            }
        }
        out
    }

    /// Whether `first-second` is a code written in two groups: upper-case letters and digits,
    /// a short first group and a longer second one, standing alone. Two groups of letters alone
    /// are a code only at three letters or more each, so `E-MAIL` and `SIGN-IN` are words.
    fn grouped(&self, first: Span, second: Span) -> bool {
        let group = |span: Span| -> Option<(usize, bool)> {
            let chars = &self.chars[span.start..span.end];
            chars
                .iter()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                .then(|| (chars.len(), chars.iter().any(char::is_ascii_digit)))
        };
        let (Some((a, a_digits)), Some((b, b_digits))) = (group(first), group(second)) else {
            return false;
        };
        let letters = self.chars[first.start..second.end]
            .iter()
            .any(char::is_ascii_uppercase);
        let shaped = (1..=4).contains(&a) && (3..=8).contains(&b) && letters;
        let worded = a_digits || b_digits || a >= 3;
        let alone = !self
            .before(first.start)
            .is_some_and(|c| c == '-' || c.is_alphanumeric())
            && !self
                .at(second.end)
                .is_some_and(|c| c == '-' || c.is_alphanumeric());
        shaped && worded && alone && self.free(first.start, second.end)
    }

    /// Whether the run at `span` is a code's digits: four to eight of them, and nothing around
    /// them that makes them a year, a price, a date, a time, a telephone number or a number
    /// labelled as something else.
    fn digits(&self, span: Span) -> bool {
        let run = &self.chars[span.start..span.end];
        if !(4..=8).contains(&run.len()) || !run.iter().all(char::is_ascii_digit) {
            return false;
        }
        let digit = |i: Option<char>| i.is_some_and(|c| c.is_ascii_digit());
        // A year.
        let value: String = run.iter().collect();
        if run.len() == 4
            && value
                .parse::<u32>()
                .is_ok_and(|y| (1900..=2099).contains(&y))
        {
            return false;
        }
        // Digits that run on past a separator or a single space: a date, a time, a price with
        // its cents, a telephone number in groups.
        let joins = |c: Option<char>| matches!(c, Some('.' | ',' | ':' | '/' | '-' | ' '));
        let before = self.before(span.start);
        let before2 = span.start.checked_sub(2).and_then(|i| self.at(i));
        let after = self.at(span.end);
        let after2 = self.at(span.end + 1);
        if (joins(before) && digit(before2)) || (joins(after) && digit(after2)) {
            return false;
        }
        // Part of a hyphenated word: `COVID-1919`.
        if before == Some('-') && before2.is_some_and(char::is_alphanumeric) {
            return false;
        }
        // A price, a telephone number's country code or area code, a number marked as one.
        let near_before = self.chars[..span.start]
            .iter()
            .rev()
            .find(|c| **c != ' ')
            .copied();
        let near_after = self.chars[span.end..].iter().find(|c| **c != ' ').copied();
        if near_before.is_some_and(|c| CURRENCY.contains(&c) || matches!(c, '#' | '+' | '('))
            || near_after.is_some_and(|c| CURRENCY.contains(&c) || c == ')')
        {
            return false;
        }
        // Labelled as an order, an account, a ticket.
        if self
            .words_before(span.start, 2)
            .iter()
            .any(|w| LABELS.contains(&w.as_str()) || w == "usd" || w == "eur" || w == "nt")
        {
            return false;
        }
        let close: String = self.chars[span.start.saturating_sub(6)..span.start]
            .iter()
            .collect();
        if CJK_LABELS.iter().any(|label| close.contains(label)) {
            return false;
        }
        if self
            .words_after(span.end, 1)
            .first()
            .is_some_and(|w| matches!(w.as_str(), "usd" | "eur" | "dollars" | "euros"))
        {
            return false;
        }
        self.free(span.start, span.end)
    }

    /// Whether `start..end` is outside a link, an address and a query: the run of text it
    /// stands in, between spaces, has no `/`, `=`, `@`, `&` or `?`.
    fn free(&self, start: usize, end: usize) -> bool {
        let mut from = start;
        while from > 0 && !self.chars[from - 1].is_whitespace() {
            from -= 1;
        }
        let mut to = end;
        while to < self.chars.len() && !self.chars[to].is_whitespace() {
            to += 1;
        }
        !self.chars[from..to]
            .iter()
            .any(|c| matches!(c, '/' | '=' | '@' | '&' | '?'))
    }
}

/// `c`, read as the ASCII it stands for where it is a full-width digit, letter or hyphen, and as
/// a space where it is any kind of space. Full-width punctuation stays as it is: `，` after a
/// code ends a clause, where `,` after a number would be its thousands.
fn narrow(c: char) -> char {
    match c {
        '\u{FF0D}'
        | '\u{FF10}'..='\u{FF19}'
        | '\u{FF21}'..='\u{FF3A}'
        | '\u{FF41}'..='\u{FF5A}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        '\n' => '\n',
        c if c.is_whitespace() => ' ',
        c => c,
    }
}

#[cfg(test)]
#[path = "otp_tests.rs"]
mod tests;
