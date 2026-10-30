//! `format=flowed` (RFC 3676): how a text part says its lines may be rewrapped, and the text
//! read back as the lines its writer meant.

/// Whether plain text may be rewrapped, and whether the flow space is real.
///
/// `delsp` is meaningless for [`Flowed::Fixed`], so it is not a separate flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flowed {
    /// `format=fixed`. Lines are the sender's line breaks.
    Fixed,
    /// `format=flowed`. A trailing space joins the next line, except on `-- `.
    Flowed {
        /// Whether the trailing space is a marker to delete on join, or a real space that also
        /// marks the join.
        delsp: Delsp,
    },
}

/// The `delsp` parameter of `format=flowed` (RFC 3676 section 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delsp {
    /// `delsp=yes`: the trailing space is a marker and is deleted on join.
    Yes,
    /// Absent or `no`: the space is real, and also marks the join.
    No,
}

/// `format=flowed` text as the lines its writer meant: one stuffing space off each line,
/// every soft line joined to the next one at its quote depth, quotes written back as `> `.
///
/// For a program that keeps text rather than draws it: the composer reopening a draft, or a
/// server that takes a plain body and knows nothing of flowed text. Unlike the reader's text parse it keeps
/// every line and every character, bidi overrides included.
pub fn unflow(text: &str, delsp: Delsp) -> String {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut joining = false;
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let (depth, body) = unquoted(raw);
        let soft = body != "-- " && body.ends_with(' ');
        let body = if soft && delsp == Delsp::Yes {
            &body[..body.len() - 1]
        } else {
            body
        };
        match out.last_mut() {
            Some((at, line)) if joining && *at == depth => line.push_str(body),
            _ => out.push((depth, body.to_owned())),
        }
        joining = soft;
    }
    out.into_iter()
        .map(|(depth, line)| match depth {
            0 => line,
            _ if line.is_empty() => ">".repeat(depth),
            _ => format!("{} {line}", ">".repeat(depth)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A flowed line's quote depth and its text, stuffing and markers off.
fn unquoted(mut raw: &str) -> (usize, &str) {
    let mut depth = 0usize;
    while let Some(rest) = raw.strip_prefix('>') {
        depth += 1;
        raw = rest;
        if raw.starts_with(' ') && raw.as_bytes().get(1) == Some(&b'>') {
            raw = &raw[1..];
        }
    }
    (depth, raw.strip_prefix(' ').unwrap_or(raw))
}
