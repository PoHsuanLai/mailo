//! URI templates at RFC 6570 level 1, which is all RFC 8620 uses.

/// Expand `template`, replacing each `{name}` with its value percent-encoded.
///
/// Level 1 is simple string expansion: every character outside the unreserved set is encoded,
/// so a blob id or a file name can never add a path segment or a query parameter. A variable
/// with no value expands to nothing, as RFC 6570 §3.2.1 says an undefined one does. An
/// unterminated `{` is copied as it stands rather than guessed at.
pub fn expand(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        if let Some((_, value)) = values.iter().find(|(n, _)| *n == name) {
            encode(value, &mut out);
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

fn encode(value: &str, out: &mut String) {
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_encoded_so_they_cannot_add_structure() {
        let url = expand(
            "https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}",
            &[
                ("accountId", "A13824"),
                ("blobId", "Gabc/../x"),
                ("name", "a b.eml"),
                ("type", "message/rfc822"),
            ],
        );
        assert_eq!(
            url,
            "https://jmap.example.com/download/A13824/Gabc%2F..%2Fx/a%20b.eml?accept=message%2Frfc822"
        );
    }

    #[test]
    fn an_unknown_variable_is_dropped_and_a_broken_brace_kept() {
        assert_eq!(expand("/a/{nope}/b", &[]), "/a//b");
        assert_eq!(expand("/a/{open", &[("open", "x")]), "/a/{open");
    }
}
