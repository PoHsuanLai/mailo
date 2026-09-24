//! ManageSieve's response grammar (RFC 5804 §4): lines of atoms, quoted strings and literals.
//!
//! One function, [`line`], that reads one whole line from the front of a buffer or says it has
//! not all arrived. A literal's octets are part of the line they appear on, so a script fetched
//! with `GETSCRIPT` is one line here however many lines it holds.

use crate::machine::ProtoError;

/// The most a single response line may hold, literals included.
///
/// A server limits scripts to far less than this (Dovecot's default is 1 MiB); the bound is what
/// stops a hostile length prefix from growing the buffer without end.
pub(super) const MAX_LINE: usize = 4 * 1024 * 1024;

/// One element of a response line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Token {
    /// A bare word: `OK`, `NO`, `BYE`, `ACTIVE`, a response code's name.
    Atom(String),
    /// A quoted string or a literal, as bytes: a script need not be valid UTF-8 to be listed.
    Text(Vec<u8>),
    /// A parenthesised response code: `(SASL "…")`, `(QUOTA/MAXSIZE)`.
    Code(Vec<Token>),
}

impl Token {
    /// The token as text, whatever kind it is. Response codes read as nothing.
    pub(super) fn text(&self) -> String {
        match self {
            Token::Atom(a) => a.clone(),
            Token::Text(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            Token::Code(_) => String::new(),
        }
    }
}

/// Why a line could not be read yet, or at all.
enum Short {
    More,
    Bad(ProtoError),
}

fn bad(what: &str) -> Short {
    Short::Bad(ProtoError::Malformed(what.to_owned()))
}

/// The first complete line of `buf` and how many bytes it took, or `None` when more are needed.
pub(super) fn line(buf: &[u8]) -> Result<Option<(Vec<Token>, usize)>, ProtoError> {
    let mut reader = Reader { buf, pos: 0 };
    match reader.tokens(Nesting::Line) {
        Ok(tokens) => Ok(Some((tokens, reader.pos))),
        Err(Short::More) if buf.len() > MAX_LINE => Err(ProtoError::Malformed(
            "a response line longer than any script".to_owned(),
        )),
        Err(Short::More) => Ok(None),
        Err(Short::Bad(e)) => Err(e),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Nesting {
    Line,
    Code,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Result<u8, Short> {
        self.buf.get(self.pos).copied().ok_or(Short::More)
    }

    fn tokens(&mut self, nesting: Nesting) -> Result<Vec<Token>, Short> {
        let mut out = Vec::new();
        loop {
            while self.peek()? == b' ' {
                self.pos += 1;
            }
            match self.peek()? {
                b'\r' if nesting == Nesting::Line => {
                    self.pos += 1;
                    if self.peek()? != b'\n' {
                        return Err(bad("CR without LF"));
                    }
                    self.pos += 1;
                    return Ok(out);
                }
                b')' if nesting == Nesting::Code => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\r' | b'\n' | b')' => return Err(bad("a line broken inside a response code")),
                b'(' if nesting == Nesting::Line => {
                    self.pos += 1;
                    out.push(Token::Code(self.tokens(Nesting::Code)?));
                }
                b'"' => out.push(Token::Text(self.quoted()?)),
                b'{' => out.push(Token::Text(self.literal()?)),
                _ => out.push(Token::Atom(self.atom()?)),
            }
        }
    }

    fn atom(&mut self) -> Result<String, Short> {
        let start = self.pos;
        loop {
            match self.peek()? {
                b' ' | b'\r' | b'\n' | b'(' | b')' | b'"' | b'{' => break,
                _ => self.pos += 1,
            }
        }
        if self.pos == start {
            return Err(bad("an empty atom"));
        }
        Ok(String::from_utf8_lossy(&self.buf[start..self.pos]).into_owned())
    }

    /// `"…"`, where `\"` and `\\` stand for themselves (RFC 5804 §4). A line break inside is
    /// malformed: a long string arrives as a literal.
    fn quoted(&mut self) -> Result<Vec<u8>, Short> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            match self.peek()? {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    out.push(self.peek()?);
                    self.pos += 1;
                }
                b'\r' | b'\n' => return Err(bad("a line break in a quoted string")),
                b => {
                    out.push(b);
                    self.pos += 1;
                }
            }
        }
    }

    /// `{n}CRLF` then exactly n octets. `{n+}` is accepted too, although only a client sends it.
    fn literal(&mut self) -> Result<Vec<u8>, Short> {
        self.pos += 1;
        let start = self.pos;
        while self.peek()?.is_ascii_digit() {
            self.pos += 1;
        }
        let digits = std::str::from_utf8(&self.buf[start..self.pos]).unwrap_or_default();
        let length: usize = digits
            .parse()
            .map_err(|_| bad("a literal with no length"))?;
        if length > MAX_LINE {
            return Err(bad("a literal longer than any script"));
        }
        if self.peek()? == b'+' {
            self.pos += 1;
        }
        for expected in *b"}\r\n" {
            if self.peek()? != expected {
                return Err(bad("a malformed literal"));
            }
            self.pos += 1;
        }
        let end = self.pos + length;
        if self.buf.len() < end {
            return Err(Short::More);
        }
        let bytes = self.buf[self.pos..end].to_vec();
        self.pos = end;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Token {
        Token::Text(s.as_bytes().to_vec())
    }

    #[test]
    fn a_line_of_every_kind_of_token_reads_whole() {
        let buf = b"NO (QUOTA/MAXSIZE) \"too \\\"big\\\" \\\\ here\"\r\nrest";
        let (tokens, used) = line(buf).unwrap().unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Atom("NO".into()),
                Token::Code(vec![Token::Atom("QUOTA/MAXSIZE".into())]),
                text("too \"big\" \\ here"),
            ]
        );
        assert_eq!(&buf[used..], b"rest");
    }

    #[test]
    fn a_literal_holds_its_line_breaks_and_the_line_goes_on_after_it() {
        let buf = b"{4}\r\na\r\nb ACTIVE\r\n\"next\"\r\n";
        let (tokens, used) = line(buf).unwrap().unwrap();
        assert_eq!(tokens, vec![text("a\r\nb"), Token::Atom("ACTIVE".into())]);
        assert_eq!(&buf[used..], b"\"next\"\r\n");
    }

    #[test]
    fn a_line_cut_anywhere_asks_for_more() {
        let whole = b"\"mailo\" ACTIVE\r\n{3}\r\nabc\r\n";
        for cut in 0..16 {
            assert_eq!(line(&whole[..cut]).unwrap(), None, "cut at {cut}");
        }
        let literal = &whole[16..];
        for cut in 0..literal.len() {
            assert_eq!(line(&literal[..cut]).unwrap(), None, "literal cut at {cut}");
        }
    }

    #[test]
    fn a_hostile_length_is_refused_rather_than_awaited() {
        assert!(line(b"{999999999999}\r\n").is_err());
        assert!(line(b"\"broken\nstring\"\r\n").is_err());
    }
}
