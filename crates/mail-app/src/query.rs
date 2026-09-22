//! What a person types into a search box, as a [`Filter`].
//!
//! `mail-domain` has had a complete query algebra since phase 1 — `From`, `To`, `Subject`,
//! `Date`, `HasAttachment`, `Read`, `Starred`, `Pinned`, `Snoozed`, `InMailbox`, and `And`/`Or`/
//! `Not` over all of them — proptested against the SQL that answers it. Every search this
//! application could make was `Filter::Text(Contains(the whole line))`. On the maildrop this
//! client is for, "from Bob about the invoice, some time last year" is a question the store could
//! already answer and nobody could ask.
//!
//! The vocabulary is the one every mail client uses, because the point is to be guessable:
//!
//! ```text
//! from:ada  to:bob  subject:lunch      a word in that field
//! is:unread is:read is:starred         state
//! is:pinned is:snoozed
//! in:inbox  in:archive in:sent         where it lives
//! in:spam   in:trash
//! has:attachment                       what it carries
//! before:2026-01-01 after:2025-12-25   when, in the reader's zone
//! -from:newsletter                     not that
//! "exact phrase"                       adjacent words, in one field
//! anything else                        full text
//! ```
//!
//! Terms are joined with `And`, which is what every client does and what everyone expects: each
//! word you add narrows the result. `Or` is deliberately absent — it reads ambiguously next to
//! `-`, and nobody types it.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use mail_domain::{DateRange, Filter, MailboxRole, ReadState, Star, TextMatch};

/// Turn a search line into a filter.
///
/// Never fails. A search box that rejects what is typed while it is being typed is unusable, so
/// a term nobody recognises — `frm:ada`, a stray colon, an unparseable date — is searched for as
/// text rather than refused. The cost of that choice is a search that finds nothing rather than
/// one that explains itself, which is the right way round while the user is still typing.
pub fn parse<Tz: TimeZone>(input: &str, zone: &Tz) -> Filter {
    let mut clauses: Vec<Filter> = Vec::new();
    let mut words: Vec<String> = Vec::new();

    for token in tokenize(input) {
        match term(&token, zone) {
            Some(filter) => clauses.push(filter),
            None => words.push(token.text),
        }
    }

    if !words.is_empty() {
        clauses.push(Filter::Text(TextMatch::Contains(words.join(" "))));
    }
    match clauses.len() {
        // An empty search is not a filter that matches nothing; it is no filter at all. The
        // caller decides what an empty box means — usually "the place you were already in".
        0 => Filter::All,
        1 => clauses.remove(0),
        _ => Filter::And(clauses),
    }
}

/// One token: its text, whether it was negated, and whether it was quoted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    text: String,
    negated: bool,
    quoted: bool,
}

/// Split on whitespace, keeping quoted runs together and noticing a leading `-`.
///
/// Hand-written because the grammar is three rules deep and a dependency for it would be a
/// dependency to keep. Quotes inside a word — `don't` — are not quotes: only a quote that opens
/// a token does.
fn tokenize(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut negated = false;
        let mut c = c;
        if c == '-' {
            // A bare `-` is a word, not a negation of nothing.
            match chars.peek() {
                Some(next) if !next.is_whitespace() => {
                    negated = true;
                    c = chars.next().unwrap_or(' ');
                }
                _ => {}
            }
        }
        let mut text = String::new();
        let mut quoted = false;
        if c == '"' {
            quoted = true;
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                text.push(c);
            }
        } else {
            text.push(c);
            // A field's value may itself be quoted: `subject:"lunch on friday"`.
            if text.ends_with(':') || !text.contains(':') {
                // fall through to the ordinary scan
            }
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                if next == '"' && text.ends_with(':') {
                    chars.next();
                    quoted = true;
                    for c in chars.by_ref() {
                        if c == '"' {
                            break;
                        }
                        text.push(c);
                    }
                    break;
                }
                text.push(next);
                chars.next();
            }
        }
        if !text.is_empty() {
            out.push(Token {
                text,
                negated,
                quoted,
            });
        }
    }
    out
}

/// The filter one token means, or `None` when it is just a word.
fn term<Tz: TimeZone>(token: &Token, zone: &Tz) -> Option<Filter> {
    let wrap = |f: Filter| {
        if token.negated {
            Filter::Not(Box::new(f))
        } else {
            f
        }
    };
    if token.quoted && !token.text.contains(':') {
        // A phrase: adjacent words, in one field. `TextMatch::Exact` is the strictest thing the
        // FTS index can honestly answer — see `Filter::Text`.
        return Some(wrap(Filter::Text(TextMatch::Exact(token.text.clone()))));
    }
    let (field, value) = token.text.split_once(':')?;
    if value.is_empty() {
        return None;
    }
    let matched = |v: &str| TextMatch::Contains(v.to_owned());
    let filter = match field.to_ascii_lowercase().as_str() {
        "from" => Filter::From(matched(value)),
        "to" => Filter::To(matched(value)),
        "subject" => Filter::Subject(matched(value)),
        "has" => match value.to_ascii_lowercase().as_str() {
            "attachment" | "attachments" | "file" => Filter::HasAttachment,
            _ => return None,
        },
        "is" => match value.to_ascii_lowercase().as_str() {
            "unread" => Filter::Read(ReadState::Unread),
            "read" => Filter::Read(ReadState::Read),
            "starred" | "flagged" => Filter::Starred(Star::Starred),
            "unstarred" => Filter::Starred(Star::Unstarred),
            "pinned" => Filter::Pinned,
            "snoozed" => Filter::Snoozed,
            _ => return None,
        },
        "in" => Filter::InMailbox(mailbox(value)?),
        // `before` is exclusive and `after` inclusive, matching `DateRange`'s own `>= from` and
        // `< to`: a day named is a whole day, and "before the 25th" should not include it.
        "before" => Filter::Date(DateRange {
            from: None,
            to: Some(midnight(value, zone)?),
        }),
        "after" => Filter::Date(DateRange {
            from: Some(midnight(value, zone)?),
            to: None,
        }),
        _ => return None,
    };
    Some(wrap(filter))
}

fn mailbox(value: &str) -> Option<MailboxRole> {
    Some(match value.to_ascii_lowercase().as_str() {
        "inbox" => MailboxRole::Inbox,
        "archive" | "all" => MailboxRole::Archive,
        "sent" => MailboxRole::Sent,
        "drafts" | "draft" => MailboxRole::Drafts,
        "spam" | "junk" => MailboxRole::Spam,
        "trash" | "bin" | "deleted" => MailboxRole::Trash,
        _ => return None,
    })
}

/// The start of a named day, in the reader's zone.
///
/// In their zone and not UTC for the reason every date in this project is: "after 2026-09-22"
/// typed in Taipei means after midnight there, which is eight hours before midnight in London.
fn midnight<Tz: TimeZone>(value: &str, zone: &Tz) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    zone.from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}
