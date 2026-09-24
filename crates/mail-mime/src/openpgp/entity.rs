//! MIME entities as bytes: where the header ends, what a field says, how a multipart splits.
//!
//! A signature covers exact bytes, so this works on the bytes themselves rather than on a
//! parsed tree that has already decoded, re-folded or normalized them away.

pub(super) use crate::stamp::fields;
use crate::stamp::split_head;

/// Deepest nesting walked when looking for a signed or encrypted part. Real mail nests a few
/// levels; a message nested thousands deep is an attack on the stack, not a message.
pub(super) const MAX_DEPTH: usize = 16;

/// One entity: its header block, and its body after the blank line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Entity<'a> {
    /// The whole entity, header and body: the bytes a signature over it covers.
    pub raw: &'a [u8],
    pub head: &'a [u8],
    pub body: &'a [u8],
}

impl<'a> Entity<'a> {
    pub fn of(raw: &'a [u8]) -> Entity<'a> {
        let (head, rest) = split_head(raw);
        let body = rest
            .strip_prefix(b"\r\n")
            .or_else(|| rest.strip_prefix(b"\n"))
            .unwrap_or(rest);
        Entity { raw, head, body }
    }

    /// The first field named `name`, unfolded and trimmed.
    pub fn header(&self, name: &str) -> Option<String> {
        fields(self.head)
            .into_iter()
            .find(|field| is_named(field, name))
            .map(value)
    }

    pub fn content_type(&self) -> ContentType {
        self.header("Content-Type")
            .map(|text| ContentType::parse(&text))
            .unwrap_or_else(ContentType::default_text)
    }

    /// The child entities, when this is a multipart with a boundary. Empty otherwise.
    pub fn children(&self) -> Vec<Entity<'a>> {
        let kind = self.content_type();
        if !kind.mime.starts_with("multipart/") {
            return Vec::new();
        }
        let Some(boundary) = kind.param("boundary") else {
            return Vec::new();
        };
        parts(self.body, boundary)
            .into_iter()
            .map(Entity::of)
            .collect()
    }
}

/// Whether `field` is named `name`, compared whole and without regard to case.
pub(super) fn is_named(field: &[u8], name: &str) -> bool {
    field_name(field).eq_ignore_ascii_case(name.as_bytes())
}

pub(super) fn field_name(field: &[u8]) -> &[u8] {
    let colon = field.iter().position(|&b| b == b':').unwrap_or(field.len());
    field[..colon].trim_ascii()
}

/// A field's value, continuation lines joined, trimmed.
pub(super) fn value(field: &[u8]) -> String {
    let colon = field.iter().position(|&b| b == b':').map_or(0, |at| at + 1);
    let unfolded: Vec<u8> = field[colon..]
        .iter()
        .copied()
        .filter(|&b| b != b'\r' && b != b'\n')
        .collect();
    String::from_utf8_lossy(&unfolded).trim().to_owned()
}

/// Whether a field describes the content rather than the message: `Content-*` and
/// `MIME-Version`. These are what move into the signed or encrypted part.
pub(super) fn is_content_field(field: &[u8]) -> bool {
    let name = field_name(field);
    name.len() > 8 && name[..8].eq_ignore_ascii_case(b"content-")
        || name.eq_ignore_ascii_case(b"MIME-Version")
}

/// `Content-Type`, parsed: the media type lower-cased, and its parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContentType {
    pub mime: String,
    params: Vec<(String, String)>,
}

impl ContentType {
    /// RFC 2045 §5.2: no `Content-Type` means plain text.
    fn default_text() -> ContentType {
        ContentType {
            mime: "text/plain".to_owned(),
            params: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> ContentType {
        let mut pieces = split_params(text).into_iter();
        let mime = pieces
            .next()
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| m.contains('/'))
            .unwrap_or_else(|| "text/plain".to_owned());
        let params = pieces
            .filter_map(|piece| {
                let (name, value) = piece.split_once('=')?;
                Some((name.trim().to_ascii_lowercase(), unquote(value.trim())))
            })
            .collect();
        ContentType { mime, params }
    }

    /// A parameter's value, by name without regard to case.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Whether this is `mime` with `protocol` naming `protocol`, both compared whole.
    pub fn is(&self, mime: &str, protocol: &str) -> bool {
        self.mime == mime
            && self
                .param("protocol")
                .is_some_and(|p| p.eq_ignore_ascii_case(protocol))
    }
}

/// `a; b="c;d"; e` into `a`, `b="c;d"`, `e`: semicolons inside quotes do not split.
fn split_params(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in text.chars() {
        match ch {
            _ if escaped => {
                current.push(ch);
                escaped = false;
            }
            '\\' if quoted => {
                current.push(ch);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ';' if !quoted => out.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    out.push(current);
    out
}

fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped || ch != '\\' {
            out.push(ch);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    out
}

/// A multipart body's parts, exactly as they lie between its boundary lines (RFC 2046 §5.1.1).
///
/// The line break before a boundary line belongs to the boundary, not to the part before it —
/// which is what makes the bytes a signature covers well defined.
pub(super) fn parts<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut out = Vec::new();
    // (start of the boundary line, start of the line after it)
    let mut open: Option<usize> = None;
    let mut line_start = 0;
    while line_start < body.len() {
        let line_end = body[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(body.len(), |at| line_start + at + 1);
        let line = &body[line_start..line_end];
        if let Some(rest) = line.strip_prefix(delimiter) {
            let rest = rest.trim_ascii_end();
            let closing = rest == b"--";
            // Transport padding is allowed after a boundary; anything else is a longer line
            // that only begins like one.
            if closing || rest.is_empty() {
                if let Some(from) = open {
                    let mut to = line_start;
                    if to > from && body[to - 1] == b'\n' {
                        to -= 1;
                        if to > from && body[to - 1] == b'\r' {
                            to -= 1;
                        }
                    }
                    out.push(&body[from..to.max(from)]);
                }
                if closing {
                    return out;
                }
                open = Some(line_end);
            }
        }
        line_start = line_end;
    }
    // No closing boundary: a truncated message. The last part runs to the end.
    if let Some(from) = open {
        out.push(&body[from..]);
    }
    out
}

/// `bytes` with every line ending a CRLF: the canonical form RFC 3156 signs.
pub(super) fn crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 40);
    let mut previous = 0u8;
    for &b in bytes {
        if b == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        previous = b;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multipart_splits_at_its_boundaries_and_the_break_before_one_is_the_boundarys() {
        let body = b"preamble\r\n--b\r\nContent-Type: text/plain\r\n\r\none\r\n--b \r\n\r\ntwo\r\n\r\n--b--\r\nepilogue\r\n";
        let got = parts(body, "b");
        assert_eq!(
            got,
            vec![
                &b"Content-Type: text/plain\r\n\r\none"[..],
                &b"\r\ntwo\r\n"[..]
            ]
        );
    }

    #[test]
    fn a_line_that_only_begins_like_a_boundary_is_content() {
        let body = b"--b\r\nx\r\n--bb\r\ny\r\n--b--\r\n";
        assert_eq!(parts(body, "b"), vec![&b"x\r\n--bb\r\ny"[..]]);
    }

    #[test]
    fn content_type_parameters_are_read_with_quotes_and_case() {
        let kind = ContentType::parse(
            "Multipart/Signed; micalg=pgp-sha256;\r\n PROTOCOL=\"application/pgp-signature\"; boundary=\"a;b\"",
        );
        assert_eq!(kind.mime, "multipart/signed");
        assert!(kind.is("multipart/signed", "application/pgp-signature"));
        assert_eq!(kind.param("boundary"), Some("a;b"));
    }

    #[test]
    fn bare_line_feeds_become_crlf_and_crlf_stays() {
        assert_eq!(crlf(b"a\nb\r\nc\n"), b"a\r\nb\r\nc\r\n");
    }
}
