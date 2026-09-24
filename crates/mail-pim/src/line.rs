//! Content lines: the grammar vCard (RFC 6350 §3.3) and iCalendar (RFC 5545 §3.1) share.
//!
//! `[group "."] name *(";" param) ":" value`, folded at 75 octets by a line break followed by
//! one space or tab. Parameters may be quoted, which is what lets a `:` or `;` appear inside
//! one, and RFC 6868 caret-escapes what quoting cannot carry.
//!
//! Older cards bend the grammar in two ways this module absorbs, so nothing above it has to:
//! vCard 2.1 writes parameters bare (`TEL;WORK;VOICE:`) and encodes values as
//! `QUOTED-PRINTABLE` in a named `CHARSET`, soft-breaking long ones with a trailing `=` rather
//! than a fold. [`lines`] undoes both, so every [`ContentLine`] it returns has a plain text value
//! and parameters that all have names.

/// One property, unfolded, its parameters parsed and its value still escaped as written.
///
/// The value is left escaped because what an escape means depends on the property: `N` splits
/// on `;` before it unescapes, `NOTE` does not split at all, and a property this module has
/// never heard of must survive being written back unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentLine {
    /// The grouping prefix some exporters put on related lines (`item1.EMAIL`, `item1.X-ABLabel`).
    pub group: Option<String>,
    /// The property name, upper-cased: names are case-insensitive, and comparing them is the
    /// only thing anything does with them.
    pub name: String,
    pub params: Vec<Param>,
    pub value: String,
}

/// One parameter and its values, in the order written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// Upper-cased, for the same reason as [`ContentLine::name`].
    pub name: String,
    /// Unquoted and caret-decoded. `TYPE=work,voice` is two values.
    pub values: Vec<String>,
}

impl ContentLine {
    /// A line with no group and no parameters.
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Self {
            group: None,
            name: name.to_ascii_uppercase(),
            params: Vec::new(),
            value: value.into(),
        }
    }

    /// The same line with one more parameter.
    pub fn with(mut self, name: &str, values: &[&str]) -> Self {
        self.params.push(Param {
            name: name.to_ascii_uppercase(),
            values: values.iter().map(|v| (*v).to_owned()).collect(),
        });
        self
    }

    /// Every value of every parameter called `name`, in order.
    pub fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.params
            .iter()
            .filter(move |p| p.name.eq_ignore_ascii_case(name))
            .flat_map(|p| p.values.iter().map(String::as_str))
    }

    /// Whether parameter `name` has `value` among its values, compared case-insensitively as
    /// whole values: `TYPE=work` has `work`, `TYPE=network` does not.
    pub fn has(&self, name: &str, value: &str) -> bool {
        self.values(name).any(|v| v.eq_ignore_ascii_case(value))
    }
}

/// Parameter values vCard 2.1 writes bare that name an encoding rather than a type.
const ENCODINGS: [&str; 5] = ["7BIT", "8BIT", "QUOTED-PRINTABLE", "BASE64", "B"];

/// Every property in `text`, unfolded and decoded. A line that does not parse is skipped: one
/// broken line in an exported address book is not a reason to lose the other contacts in it.
pub fn lines(text: &str) -> Vec<ContentLine> {
    unfold(text)
        .iter()
        .filter_map(|line| parse(line))
        .map(decode_value)
        .collect()
}

/// Undo folding: a physical line that begins with a space or a tab continues the one before it,
/// less that one character (RFC 6350 §3.2). Blank lines are dropped.
///
/// Also joins vCard 2.1's quoted-printable soft breaks — a value encoded that way that ends in
/// `=` continues on the next physical line, which does *not* begin with a space.
///
/// Accepts `\r\n`, a bare `\n`, or a bare `\r` between lines: files that have been through an
/// editor or another platform's tools arrive with any of the three.
pub fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut soft_break = false;
    for physical in text.split('\n').flat_map(|l| l.split('\r')) {
        // A CRLF split on both characters leaves an empty piece between them, and a blank line
        // means nothing in either format. Neither may end a soft break.
        if physical.is_empty() {
            continue;
        }
        let continues = physical.starts_with([' ', '\t']);
        match out.last_mut() {
            Some(last) if soft_break => {
                last.pop(); // the `=`
                last.push_str(physical);
            }
            Some(last) if continues => last.push_str(&physical[1..]),
            _ => out.push(physical.to_owned()),
        }
        soft_break = out
            .last()
            .is_some_and(|l| l.ends_with('=') && quoted_printable(l));
    }
    out
}

/// Whether a raw line declares a quoted-printable value, by its parameters as whole tokens.
fn quoted_printable(line: &str) -> bool {
    let Some((head, _)) = split_head(line) else {
        return false;
    };
    split_outside_quotes(head, ';').iter().skip(1).any(|p| {
        let value = p.split_once('=').map_or(p.as_str(), |(_, v)| v);
        value
            .split(',')
            .any(|v| v.trim_matches('"').eq_ignore_ascii_case("QUOTED-PRINTABLE"))
    })
}

/// One unfolded line, or `None` when it has no `:` or no name.
pub fn parse(line: &str) -> Option<ContentLine> {
    let (head, value) = split_head(line)?;
    let mut parts = split_outside_quotes(head, ';').into_iter();
    let full_name = parts.next()?;
    let (group, name) = match full_name.split_once('.') {
        Some((group, name)) => (Some(group.to_owned()), name),
        None => (None, full_name.as_str()),
    };
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let params = parts.filter(|p| !p.is_empty()).map(|p| param(&p)).collect();
    Some(ContentLine {
        group,
        name: name.to_ascii_uppercase(),
        params,
        value: value.to_owned(),
    })
}

/// One `name=value,value` parameter, or a vCard 2.1 bare one.
fn param(raw: &str) -> Param {
    match raw.split_once('=') {
        Some((name, values)) => Param {
            name: name.trim().to_ascii_uppercase(),
            values: split_outside_quotes(values, ',')
                .iter()
                .map(|v| uncaret(unquote(v.trim())))
                .collect(),
        },
        None => {
            let value = raw.trim();
            let name = if ENCODINGS.iter().any(|e| e.eq_ignore_ascii_case(value)) {
                "ENCODING"
            } else {
                "TYPE"
            };
            Param {
                name: name.to_owned(),
                values: vec![value.to_owned()],
            }
        }
    }
}

/// The part before the first `:` outside quotes, and the rest.
fn split_head(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// `text` split on `sep` wherever it is not inside double quotes.
fn split_outside_quotes(text: &str, sep: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    for ch in text.chars() {
        match ch {
            c if c == sep && !quoted => out.push(String::new()),
            c => {
                if c == '"' {
                    quoted = !quoted;
                }
                if let Some(part) = out.last_mut() {
                    part.push(c);
                }
            }
        }
    }
    out
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

/// RFC 6868: `^n` is a line break, `^'` a double quote, `^^` a caret. Any other `^` is itself.
fn uncaret(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match (ch, chars.peek()) {
            ('^', Some('n' | 'N')) => {
                chars.next();
                out.push('\n');
            }
            ('^', Some('\'')) => {
                chars.next();
                out.push('"');
            }
            ('^', Some('^')) => {
                chars.next();
                out.push('^');
            }
            (c, _) => out.push(c),
        }
    }
    out
}

fn caret(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '^' => out.push_str("^^"),
            '\n' => out.push_str("^n"),
            '"' => out.push_str("^'"),
            c => out.push(c),
        }
    }
    out
}

/// Decode a vCard 2.1 quoted-printable value in its `CHARSET`, and drop the two parameters that
/// described it: the value they described no longer exists.
fn decode_value(mut line: ContentLine) -> ContentLine {
    if !line.has("ENCODING", "QUOTED-PRINTABLE") {
        return line;
    }
    let bytes = from_quoted_printable(&line.value);
    let charset = line.values("CHARSET").next().unwrap_or("UTF-8").to_owned();
    let encoding =
        encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(&bytes);
    line.value = text.into_owned();
    line.params
        .retain(|p| p.name != "ENCODING" && p.name != "CHARSET");
    line
}

/// `=XX` is the byte XX; a trailing `=` is a soft break already joined by [`unfold`]; an `=`
/// followed by anything that is not two hex digits is itself, which is what exporters that
/// forget to encode `=` produce.
fn from_quoted_printable(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| {
                std::str::from_utf8(h)
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
            });
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Undo text escaping (RFC 6350 §3.4): `\n` or `\N` is a line break, and `\\`, `\,`, `\;` are
/// the character. A backslash before anything else is dropped, keeping the character, which is
/// how version 3.0 exporters that escape `:` are read.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Escape text for a value: the inverse of [`unescape`].
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Split a structured value (`N`, `ORG`, `ADR`) on unescaped `sep`, unescaping each part.
pub fn split_escaped(value: &str, sep: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(part) = parts.last_mut() {
                    part.push('\\');
                    if let Some(next) = chars.next() {
                        part.push(next);
                    }
                }
            }
            c if c == sep => parts.push(String::new()),
            c => {
                if let Some(part) = parts.last_mut() {
                    part.push(c);
                }
            }
        }
    }
    parts.iter().map(|p| unescape(p)).collect()
}

/// The octets a physical line may hold before it is folded (RFC 6350 §3.2).
const FOLD_AT: usize = 75;

/// One line as it goes on the wire: named, parameterised, folded, and ended with CRLF.
pub fn write(line: &ContentLine) -> String {
    let mut logical = String::new();
    if let Some(group) = &line.group {
        logical.push_str(group);
        logical.push('.');
    }
    logical.push_str(&line.name);
    for param in &line.params {
        logical.push(';');
        logical.push_str(&param.name);
        logical.push('=');
        let values: Vec<String> = param.values.iter().map(|v| param_value(v)).collect();
        logical.push_str(&values.join(","));
    }
    logical.push(':');
    logical.push_str(&line.value);
    fold(&logical)
}

/// A parameter value, quoted when it holds a character that would otherwise end it.
fn param_value(value: &str) -> String {
    let encoded = caret(value);
    if encoded.contains([':', ';', ',']) {
        format!("\"{encoded}\"")
    } else {
        encoded
    }
}

/// Fold at [`FOLD_AT`] octets, never inside a UTF-8 sequence, and end with CRLF.
fn fold(logical: &str) -> String {
    let mut out = String::with_capacity(logical.len() + logical.len() / FOLD_AT * 3 + 2);
    let mut width = 0;
    for ch in logical.chars() {
        if width + ch.len_utf8() > FOLD_AT {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += ch.len_utf8();
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_is_undone_whichever_whitespace_and_line_ending_it_used() {
        const CASES: &[(&str, &[&str])] = &[
            ("NOTE:one\r\n two\r\n", &["NOTE:onetwo"]),
            ("NOTE:one\n\ttwo\n", &["NOTE:onetwo"]),
            ("NOTE:one\r  two\r", &["NOTE:one two"]),
            ("A:1\r\n\r\nB:2", &["A:1", "B:2"]),
        ];
        for (input, expected) in CASES {
            assert_eq!(unfold(input), *expected, "{input:?}");
        }
    }

    #[test]
    fn a_quoted_printable_soft_break_joins_the_next_line() {
        let text = "NOTE;ENCODING=QUOTED-PRINTABLE:first=\r\n second=\r\nthird\r\nFN:x\r\n";
        assert_eq!(
            unfold(text),
            ["NOTE;ENCODING=QUOTED-PRINTABLE:first secondthird", "FN:x"]
        );
    }

    #[test]
    fn a_trailing_equals_sign_on_a_plain_value_is_just_a_character() {
        assert_eq!(unfold("NOTE:a=\r\nFN:x\r\n"), ["NOTE:a=", "FN:x"]);
    }

    #[test]
    fn parameters_keep_colons_and_semicolons_inside_quotes() {
        let line = parse(r#"item1.EMAIL;TYPE="work,x;y:z";PREF=1:a@b.test"#).unwrap();
        assert_eq!(line.group.as_deref(), Some("item1"));
        assert_eq!(line.name, "EMAIL");
        assert_eq!(line.value, "a@b.test");
        assert_eq!(
            line.params,
            [
                Param {
                    name: "TYPE".into(),
                    values: vec!["work,x;y:z".into()]
                },
                Param {
                    name: "PREF".into(),
                    values: vec!["1".into()]
                }
            ]
        );
    }

    #[test]
    fn bare_parameters_are_types_unless_they_name_an_encoding() {
        let line = parse("TEL;WORK;VOICE;QUOTED-PRINTABLE:1").unwrap();
        assert_eq!(line.values("TYPE").collect::<Vec<_>>(), ["WORK", "VOICE"]);
        assert!(line.has("ENCODING", "quoted-printable"));
    }

    #[test]
    fn caret_escapes_are_decoded_in_parameter_values() {
        let line = parse("X-A;LABEL=\"line^none ^'q^' ^^\":v").unwrap();
        assert_eq!(line.values("LABEL").next(), Some("line\none \"q\" ^"));
    }

    #[test]
    fn quoted_printable_is_decoded_in_its_charset() {
        const CASES: &[(&str, &str)] = &[
            (
                "FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Ren=C3=A9e",
                "Renée",
            ),
            ("FN;CHARSET=ISO-8859-1;QUOTED-PRINTABLE:Ren=E9e", "Renée"),
            ("FN;ENCODING=QUOTED-PRINTABLE:a=3Db=zz", "a=b=zz"),
        ];
        for (input, expected) in CASES {
            let line = &lines(input)[0];
            assert_eq!(line.value, *expected, "{input}");
            assert!(line.params.is_empty(), "{input}: {:?}", line.params);
        }
    }

    #[test]
    fn a_line_with_no_colon_or_no_name_is_not_a_property() {
        for input in ["just text", ":value", ";X=1:v"] {
            assert_eq!(parse(input), None, "{input}");
        }
    }

    #[test]
    fn escaping_round_trips_every_special_character() {
        let text = "a\\b, c; d\ne";
        assert_eq!(escape(text), "a\\\\b\\, c\\; d\\ne");
        assert_eq!(unescape(&escape(text)), text);
    }

    #[test]
    fn a_structured_value_splits_only_on_unescaped_separators() {
        assert_eq!(
            split_escaped("Doe\\;Jr;Jane;;Dr.;", ';'),
            ["Doe;Jr", "Jane", "", "Dr.", ""]
        );
    }

    #[test]
    fn a_long_line_folds_at_75_octets_without_splitting_a_character() {
        let value = "é".repeat(80);
        let written = write(&ContentLine::new("NOTE", value.clone()));
        for physical in written.split("\r\n") {
            assert!(physical.len() <= FOLD_AT, "{} octets", physical.len());
        }
        assert_eq!(unfold(&written), [format!("NOTE:{value}")]);
    }

    #[test]
    fn a_written_line_parses_back_to_itself() {
        let line = ContentLine::new("EMAIL", "a@b.test")
            .with("TYPE", &["work", "a:b"])
            .with("X-Q", &["say \"hi\"\nthere"]);
        let written = write(&line);
        assert_eq!(lines(&written), [line]);
    }
}
