//! When a rule may send mail on its own: the one question every automatic send asks.
//!
//! A rule that answers or forwards mail speaks without a person reading the message first, so it
//! must never speak to a machine, and never twice to one person in a row (RFC 3834). Reply and
//! Forward both ask [`guard`] before they make a draft; nothing else decides.
//!
//! Refused, whatever the rule says:
//! - mail with an `Auto-Submitted` header other than `no`;
//! - `Precedence: bulk`, `list` or `junk`;
//! - any `List-*` header (a mailing list's, or an unsubscribe link's);
//! - a `Return-Path` that is null (`<>`) or missing: a bounce, or mail with no one to answer;
//! - mail from the user's own addresses;
//! - for a Reply, a sender that takes no answers (`noreply@`, `mailer-daemon@`) and one already
//!   answered by this rule within its days;
//! - mail whose bytes are not here yet, which cannot be read for any of the above.
//!
//! And when the mail has just arrived (not when the person picked it and asked), mail dated
//! before the rule was made: a re-download, or a mailbox fetched anew after its UIDVALIDITY
//! changed, never sends the history.

use crate::error::CoreError;
use crate::notify::Own;
use chrono::{DateTime, TimeDelta, Utc};
use mail_domain::{Message, RuleAction};
use mail_store::SqliteStore;
use mail_store::rules::Left;

/// What the rule does with the mail it is guarded for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sends {
    Reply,
    Forward,
}

/// Why a rule did not send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The message was itself made by a machine (`Auto-Submitted`).
    AutoSubmitted,
    /// `Precedence: bulk`, `list` or `junk`.
    Bulk,
    /// It carries a `List-*` header.
    ListMail,
    /// No `Return-Path`: nothing says where an answer would go.
    NoReturnPath,
    /// `Return-Path: <>`: a bounce, which no one reads.
    NullReturnPath,
    /// From one of the user's own addresses.
    Own,
    /// From no one, or from an address that takes no answers.
    NoSender,
    /// Only its headers are here: its bytes cannot be read yet.
    NotHere,
    /// Dated before the rule was made.
    BeforeTheRule,
    /// This rule answered the sender within its days already.
    Recently,
}

/// The header names in `raw`'s header section, as written.
fn header_names(raw: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    for line in raw.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line[0] == b' ' || line[0] == b'\t' {
            continue;
        }
        if let Some(colon) = line.iter().position(|b| *b == b':') {
            names.push(String::from_utf8_lossy(&line[..colon]).trim().to_owned());
        }
    }
    names
}

/// Whether the headers and sender of a message allow a rule to `sends` for it, with no memory of
/// what was sent before: the part of [`guard`] that is a function of the message alone.
pub fn headers_allow(
    raw: Option<&[u8]>,
    from: &str,
    own: &Own,
    sends: Sends,
) -> Result<(), Refusal> {
    let from = from.trim();
    if from.is_empty() {
        return Err(Refusal::NoSender);
    }
    if own.includes(from) {
        return Err(Refusal::Own);
    }
    if sends == Sends::Reply && mail_store::no_reply(from) {
        return Err(Refusal::NoSender);
    }
    let Some(raw) = raw else {
        return Err(Refusal::NotHere);
    };
    let header = |name: &str| mail_mime::archive::header(raw, name);
    if let Some(auto) = header("Auto-Submitted")
        && !auto.trim().eq_ignore_ascii_case("no")
    {
        return Err(Refusal::AutoSubmitted);
    }
    if let Some(precedence) = header("Precedence")
        && matches!(
            precedence.trim().to_ascii_lowercase().as_str(),
            "bulk" | "list" | "junk"
        )
    {
        return Err(Refusal::Bulk);
    }
    if header_names(raw)
        .iter()
        .any(|name| name.len() > 5 && name[..5].eq_ignore_ascii_case("list-"))
    {
        return Err(Refusal::ListMail);
    }
    match header("Return-Path") {
        None => Err(Refusal::NoReturnPath),
        Some(path) => {
            let path = path
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .trim();
            if path.is_empty() {
                Err(Refusal::NullReturnPath)
            } else {
                Ok(())
            }
        }
    }
}

/// How the mail came to be acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occasion {
    /// It has just arrived: only mail dated after the rule was made.
    Arrival,
    /// The person picked it and asked (Apply Rules): older mail too.
    Asked,
}

/// Whether the rule behind `left` may send for `message`, and, for a Reply, that it has now
/// answered the sender. The one function every automatic send goes through.
pub fn guard(
    store: &SqliteStore,
    left: &Left,
    message: &Message,
    own: &Own,
    occasion: Occasion,
    now: DateTime<Utc>,
) -> Result<(), Refusal> {
    let sends = match left.action {
        RuleAction::Reply(_) => Sends::Reply,
        RuleAction::Forward(_) => Sends::Forward,
        // Saying it arrived sends nothing.
        _ => return Ok(()),
    };
    if occasion == Occasion::Arrival && message.date < left.made {
        return Err(Refusal::BeforeTheRule);
    }
    let raw = crate::export::raw_message(store, message.id)
        .or_else_none("a rule could not read a message");
    headers_allow(raw.as_deref(), &message.from.email, own, sends)?;
    if sends == Sends::Reply {
        let within = TimeDelta::days(i64::from(left.reply_days));
        let free = store
            .claim_reply(left.rule, &message.from.email, now, within)
            .unwrap_or(false);
        if !free {
            return Err(Refusal::Recently);
        }
    }
    Ok(())
}

trait OrElseNone {
    fn or_else_none(self, what: &str) -> Option<Vec<u8>>;
}

impl OrElseNone for Result<Option<Vec<u8>>, CoreError> {
    fn or_else_none(self, what: &str) -> Option<Vec<u8>> {
        match self {
            Ok(raw) => raw,
            Err(why) => {
                log::warn!("{what}: {why}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> Own {
        Own::new(["me@example.test"])
    }

    fn mail(headers: &str) -> Vec<u8> {
        format!("{headers}Subject: Hello\r\n\r\nBody.\r\n").into_bytes()
    }

    const PATH: &str = "Return-Path: <ada@example.test>\r\n";

    #[test]
    fn what_a_rule_may_send_for_is_a_table_of_headers_and_senders() {
        type Case<'a> = (&'a str, Vec<u8>, &'a str, Sends, Result<(), Refusal>);
        let cases: Vec<Case> = vec![
            (
                "a person's mail",
                mail(PATH),
                "ada@example.test",
                Sends::Reply,
                Ok(()),
            ),
            (
                "a person's mail, forwarded",
                mail(PATH),
                "ada@example.test",
                Sends::Forward,
                Ok(()),
            ),
            (
                "Auto-Submitted: no is a person's",
                mail(&format!("{PATH}Auto-Submitted: no\r\n")),
                "ada@example.test",
                Sends::Reply,
                Ok(()),
            ),
            (
                "auto-replied",
                mail(&format!("{PATH}Auto-Submitted: auto-replied\r\n")),
                "ada@example.test",
                Sends::Reply,
                Err(Refusal::AutoSubmitted),
            ),
            (
                "auto-generated, forwarded too",
                mail(&format!("{PATH}Auto-Submitted: AUTO-GENERATED\r\n")),
                "ada@example.test",
                Sends::Forward,
                Err(Refusal::AutoSubmitted),
            ),
            (
                "bulk",
                mail(&format!("{PATH}Precedence: bulk\r\n")),
                "ada@example.test",
                Sends::Reply,
                Err(Refusal::Bulk),
            ),
            (
                "list",
                mail(&format!("{PATH}Precedence: List\r\n")),
                "ada@example.test",
                Sends::Forward,
                Err(Refusal::Bulk),
            ),
            (
                "junk",
                mail(&format!("{PATH}Precedence: junk\r\n")),
                "ada@example.test",
                Sends::Reply,
                Err(Refusal::Bulk),
            ),
            (
                "first class precedence is nothing",
                mail(&format!("{PATH}Precedence: first-class\r\n")),
                "ada@example.test",
                Sends::Reply,
                Ok(()),
            ),
            (
                "a list header",
                mail(&format!(
                    "{PATH}List-Unsubscribe: <mailto:u@list.example>\r\n"
                )),
                "ada@example.test",
                Sends::Reply,
                Err(Refusal::ListMail),
            ),
            (
                "another list header",
                mail(&format!("{PATH}list-id: <x.list.example>\r\n")),
                "ada@example.test",
                Sends::Forward,
                Err(Refusal::ListMail),
            ),
            (
                "no return path",
                mail(""),
                "ada@example.test",
                Sends::Reply,
                Err(Refusal::NoReturnPath),
            ),
            (
                "a null return path",
                mail("Return-Path: <>\r\n"),
                "ada@example.test",
                Sends::Forward,
                Err(Refusal::NullReturnPath),
            ),
            (
                "our own address",
                mail(PATH),
                "ME@example.test",
                Sends::Reply,
                Err(Refusal::Own),
            ),
            (
                "our own address, forwarded",
                mail(PATH),
                "me@example.test",
                Sends::Forward,
                Err(Refusal::Own),
            ),
            (
                "no sender",
                mail(PATH),
                "",
                Sends::Reply,
                Err(Refusal::NoSender),
            ),
            (
                "a machine's address",
                mail(PATH),
                "noreply@bank.example",
                Sends::Reply,
                Err(Refusal::NoSender),
            ),
            (
                "a machine's mail is forwarded",
                mail(PATH),
                "noreply@bank.example",
                Sends::Forward,
                Ok(()),
            ),
        ];
        for (name, raw, from, sends, want) in cases {
            assert_eq!(
                headers_allow(Some(&raw), from, &own(), sends),
                want,
                "{name}"
            );
        }
        assert_eq!(
            headers_allow(None, "ada@example.test", &own(), Sends::Reply),
            Err(Refusal::NotHere),
            "headers only"
        );
    }

    #[test]
    fn a_header_in_the_quoted_body_is_not_the_messages() {
        let raw =
            format!("{PATH}Subject: Re\r\n\r\nList-Id: <x>\r\nAuto-Submitted: auto-replied\r\n")
                .into_bytes();
        assert_eq!(
            headers_allow(Some(&raw), "ada@example.test", &own(), Sends::Reply),
            Ok(())
        );
    }
}
