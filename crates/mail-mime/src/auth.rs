//! What the receiving server says it checked about a message's sender: `Authentication-Results`
//! (RFC 8601), read for SPF (RFC 7208), DKIM (RFC 6376) and DMARC (RFC 7489).
//!
//! Pure, and read on demand from the stored raw message, like the list headers: the header is
//! in the bytes the server sent, and nothing about it is kept in a column.
//!
//! **Whose word it is decides everything.** The field is plain text, and anyone can write one
//! into a message before sending it. What makes a result worth showing is the server that wrote
//! it, named by the field's first word, its authserv-id. RFC 8601 §5 has the receiving server
//! remove any field that claims its own authserv-id and did not come from inside its own
//! boundary; §7.1 says a reader should otherwise ignore fields it cannot tie to a server it
//! trusts. So one field is read and the rest are ignored:
//! - with the receiving provider's domains known ([`Receiver::Domains`]), the topmost field whose
//!   authserv-id is one of them. A receiving server prepends its fields, so everything a sender
//!   wrote lies below it, and one that claims the provider's name was removed on the way in (§5);
//! - with nothing known ([`Receiver::Topmost`]), the topmost field, whatever it names.
//!
//! Every other field is ignored whatever it says: a `dkim=pass` further down counts for nothing.

use mail_parser::{HeaderName, MessageParser};

/// Which server's `Authentication-Results` to believe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiver {
    /// The provider that received the mail is not known, so only the topmost field is read.
    Topmost,
    /// The provider's domains, lower case: an authserv-id is theirs when it is one of these or a
    /// name under one of them, compared by whole labels (`mx.example.com` is under `example.com`;
    /// `evil-example.com` is not).
    Domains(Vec<String>),
}

/// What one check came to, in RFC 8601's result words (§2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    /// SPF's "probably not authorised".
    SoftFail,
    Neutral,
    /// The check found nothing to check: no SPF record, no DKIM signature, no DMARC policy.
    None,
    /// A transient failure while checking (a DNS timeout).
    TempError,
    /// The published record could not be read.
    PermError,
    /// DKIM's "the signature is good, and local policy refused it".
    Policy,
    /// A result word this reader does not know.
    Unknown,
}

/// One check's outcome, and the domain it was about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub verdict: Verdict,
    /// The domain the check was about, lower case, when the field names one: SPF's
    /// `smtp.mailfrom` (or `smtp.helo`), DKIM's `header.d` (or `header.i`), DMARC's
    /// `header.from`. A DKIM pass is only as good as whose signature it was.
    pub domain: Option<String>,
}

/// The trusted field's results. A method it does not mention is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthResults {
    /// Who wrote it: the field's authserv-id, lower case. `None` when the field starts with a
    /// result rather than a name, which only [`Receiver::Topmost`] accepts.
    pub authserv_id: Option<String>,
    pub spf: Option<Check>,
    /// With several signatures, a passing one if there is one, else the first.
    pub dkim: Option<Check>,
    pub dmarc: Option<Check>,
}

/// The results of the one field in `raw`'s headers that `receiver` trusts, or `None` when no
/// field is trusted or the trusted one cannot be read. The body is not read.
pub fn authentication_results(raw: &[u8], receiver: &Receiver) -> Option<AuthResults> {
    // Raw, not text: the field is structured and never carries encoded words. Decoding them
    // would let a value inside a comment or a quoted string turn into `; dkim=pass`.
    let parser = MessageParser::new()
        .header_raw(HeaderName::AuthenticationResults)
        .default_header_ignore();
    let message = parser.parse_headers(raw)?;
    // In the order they appear: the topmost is the last one added.
    let mut fields = message
        .header_values(HeaderName::AuthenticationResults)
        .filter_map(|value| value.as_text().map(unfold));
    let field = match receiver {
        Receiver::Topmost => fields.next()?,
        Receiver::Domains(domains) => fields.find(|field| {
            authserv_id(field).is_some_and(|id| domains.iter().any(|d| under(&id, d)))
        })?,
    };
    results(&field)
}

/// Whether `host` is `domain` or a name under it, by whole labels.
fn under(host: &str, domain: &str) -> bool {
    let domain = domain.trim_end_matches('.');
    !domain.is_empty()
        && (host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|rest| rest.ends_with('.')))
}

/// A folded field on one line.
fn unfold(value: &str) -> String {
    value.replace(['\r', '\n', '\t'], " ")
}

/// The field's authserv-id, lower case, or `None` when it has none: it opens with a result.
fn authserv_id(field: &str) -> Option<String> {
    let bare = tighten(&strip_comments(field));
    let first = split_outside_quotes(&bare, ';').into_iter().next()?;
    let word = first.split_whitespace().next()?;
    if word.contains('=') {
        return None;
    }
    Some(unquote(word).to_ascii_lowercase())
}

/// Read one field. `None` when it is not a field this reader understands: an authres-version
/// other than 1 (§2.2 has a reader ignore a version it does not know), or nothing parseable.
fn results(field: &str) -> Option<AuthResults> {
    let bare = tighten(&strip_comments(field));
    let mut parts = split_outside_quotes(&bare, ';').into_iter();
    let head = parts.next()?;
    let mut words = head.split_whitespace();
    let first = words.next()?;
    let mut out = AuthResults {
        authserv_id: None,
        spf: None,
        dkim: None,
        dmarc: None,
    };
    if first.contains('=') {
        // No authserv-id: the first part is already a result.
        resinfo(&head, &mut out);
    } else {
        out.authserv_id = Some(unquote(first).to_ascii_lowercase());
        if let Some(version) = words.next()
            && version.trim() != "1"
        {
            return None;
        }
    }
    for part in parts {
        let part = part.trim();
        if part.eq_ignore_ascii_case("none") {
            // `none`: nothing was checked (§2.2).
            continue;
        }
        resinfo(part, &mut out);
    }
    Some(out)
}

/// One `method=result ptype.property=value …` into `out`.
fn resinfo(part: &str, out: &mut AuthResults) {
    let Some((method, verdict, rest)) = method_spec(part) else {
        return;
    };
    let props = properties(rest);
    let prop = |name: &str| {
        props
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| domain_of(value))
    };
    let (slot, domain) = match method.as_str() {
        "spf" => (
            &mut out.spf,
            prop("smtp.mailfrom").or_else(|| prop("smtp.helo")),
        ),
        "dkim" => (&mut out.dkim, prop("header.d").or_else(|| prop("header.i"))),
        "dmarc" => (&mut out.dmarc, prop("header.from")),
        _ => return,
    };
    let check = Check {
        verdict,
        domain: domain.filter(|d| !d.is_empty()),
    };
    // A second result for the same method (several DKIM signatures) replaces the first only
    // when it passes and the first did not.
    match slot {
        Some(had) if had.verdict == Verdict::Pass || verdict != Verdict::Pass => {}
        _ => *slot = Some(check),
    }
}

/// `method [/ version] = result`, lower case, and what follows it.
fn method_spec(part: &str) -> Option<(String, Verdict, &str)> {
    let (left, right) = part.split_once('=')?;
    let method = left.split('/').next()?.trim().to_ascii_lowercase();
    if method.is_empty() || !method.chars().all(keyword_char) {
        return None;
    }
    let right = right.trim_start();
    let end = right
        .find(|c: char| !keyword_char(c))
        .unwrap_or(right.len());
    let (result, rest) = right.split_at(end);
    Some((method, verdict(result), rest))
}

fn keyword_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

fn verdict(word: &str) -> Verdict {
    match word.to_ascii_lowercase().as_str() {
        "pass" => Verdict::Pass,
        // `hardfail` is an older spelling of SPF's `fail`.
        "fail" | "hardfail" => Verdict::Fail,
        "softfail" => Verdict::SoftFail,
        "neutral" => Verdict::Neutral,
        "none" => Verdict::None,
        "temperror" => Verdict::TempError,
        "permerror" => Verdict::PermError,
        "policy" => Verdict::Policy,
        _ => Verdict::Unknown,
    }
}

/// `ptype.property=value` pairs, key lower case. `reason=` is a reasonspec, not a property, and
/// is skipped.
fn properties(rest: &str) -> Vec<(String, String)> {
    split_outside_quotes(rest, ' ')
        .into_iter()
        .filter_map(|word| {
            let (key, value) = word.split_once('=')?;
            let key = key.trim().to_ascii_lowercase();
            key.contains('.')
                .then(|| (key, unquote(value.trim()).to_owned()))
        })
        .collect()
}

/// The domain a property value names: after the `@` of an address, else the value itself.
fn domain_of(value: &str) -> String {
    value
        .rsplit_once('@')
        .map_or(value, |(_, domain)| domain)
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn unquote(word: &str) -> &str {
    word.strip_prefix('"')
        .and_then(|w| w.strip_suffix('"'))
        .unwrap_or(word)
}

/// `field` with every comment (RFC 5322 §3.2.2, nested, with quoted pairs) made a space. Quoted
/// strings are kept as they are, parentheses in them included. An unclosed comment runs to the
/// end.
fn strip_comments(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut depth = 0usize;
    let mut quoted = false;
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        match (depth, quoted, c) {
            (0, true, '\\') => {
                out.push(c);
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            (0, _, '"') => {
                quoted = !quoted;
                out.push(c);
            }
            (0, true, _) => out.push(c),
            (0, false, '(') => {
                depth = 1;
                out.push(' ');
            }
            (0, false, _) => out.push(c),
            (_, _, '\\') => {
                chars.next();
            }
            (_, _, '(') => depth += 1,
            (_, _, ')') => depth -= 1,
            _ => {}
        }
    }
    out
}

/// `text` with the space around each `=` and `/` outside quoted strings taken out, so
/// `dkim = pass header.d = example.com` reads as `dkim=pass header.d=example.com`: the grammar
/// allows folding whitespace on both sides of either.
fn tighten(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quoted = false;
    let mut escaped = false;
    let mut joined = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
            out.push(c);
            continue;
        }
        match c {
            '\\' if quoted => {
                escaped = true;
                out.push(c);
            }
            '"' => {
                quoted = !quoted;
                out.push(c);
            }
            '=' | '/' if !quoted => {
                out.truncate(out.trim_end_matches(' ').len());
                out.push(c);
                joined = true;
                continue;
            }
            ' ' if !quoted && joined => continue,
            c => out.push(c),
        }
        joined = false;
    }
    out
}

/// `text` split at `at` where it is not inside a quoted string, empty pieces dropped.
fn split_outside_quotes(text: &str, at: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut piece = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            escaped = false;
            piece.push(c);
            continue;
        }
        match c {
            '\\' if quoted => {
                escaped = true;
                piece.push(c);
            }
            '"' => {
                quoted = !quoted;
                piece.push(c);
            }
            c if c == at && !quoted => out.push(std::mem::take(&mut piece)),
            c => piece.push(c),
        }
    }
    out.push(piece);
    out.into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}
