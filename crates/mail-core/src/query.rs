//! What a person types into a search box, as a [`Filter`].
//!
//! `mail-domain` has a complete query algebra — `From`, `To`, `Subject`, `Date`,
//! `HasAttachment`, `Read`, `Starred`, `Pinned`, `Snoozed`, `InMailbox`, and `And`/`Or`/`Not`
//! over all of them — proptested against the SQL that answers it. This turns a typed line into
//! it, so "from Bob about the invoice, some time last year" is a question a person can ask.
//!
//! The vocabulary is the one every mail client uses, because the point is to be guessable:
//!
//! ```text
//! from:ada  to:bob  subject:lunch      a word in that field
//! is:unread is:read is:starred         state
//! is:pinned is:snoozed is:kept       is:kept is starred or pinned
//! in:inbox  in:archive in:sent         where it lives
//! in:spam   in:trash
//! has:attachment                       what it carries
//! label:travel                        a label the server gave it
//! category:promotions                 one of Gmail's inbox tabs (primary, social, promotions,
//!                                     updates, forums); only a Gmail server search answers it
//! before:2026-01-01 after:2025-12-25   when, in the reader's zone
//! -from:newsletter                     not that
//! "exact phrase"                       adjacent words, in one field
//! anything else                        full text
//! ```
//!
//! Terms are joined with `And`, which is what every client does and what everyone expects: each
//! word you add narrows the result. `Or` is deliberately absent — it reads ambiguously next to
//! `-`, and nobody types it.

use crate::error::Logged;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use mail_domain::{Category, DateRange, Filter, LabelId, MailboxRole, ReadState, Star, TextMatch};

/// Every label name in the store, with the label that bears it.
///
/// The other half of `label:`, kept next to the parser it feeds rather than at each surface,
/// because that is where the two drifted: `mailo search` passed a resolver and the window passed
/// `parse`, whose resolver knows no names. `label:travel` therefore worked in the terminal and,
/// in the window, became a full-text search for the literal string — no results and no error.
///
/// A name can appear more than once: `UNIQUE (account, name)` is per account, so the same word
/// on two accounts is two labels, and someone typing it means both.
pub fn known_labels(store: &mail_store::SqliteStore) -> Vec<(String, LabelId)> {
    let accounts: Vec<porter_core::AccountId> = store
        .list_accounts()
        .or_log_default("the accounts could not be read for their labels")
        .into_iter()
        .map(|account| account.id)
        .collect();
    accounts
        .into_iter()
        .filter_map(|account| {
            store
                .labels(account)
                .or_log("an account's labels could not be read")
        })
        .flatten()
        .map(|l| (l.name, l.id))
        .collect()
}

/// Resolve one name against [`known_labels`], for [`parse_with`].
///
/// Case-insensitive, because nobody types `label:Travel` to mean something other than
/// `label:travel`.
pub fn named<'a>(index: &'a [(String, LabelId)]) -> impl Fn(&str) -> Vec<LabelId> + 'a {
    move |name| {
        index
            .iter()
            .filter(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, id)| *id)
            .collect()
    }
}

/// Turn a search line into a filter.
///
/// Never fails. A search box that rejects what is typed while it is being typed is unusable, so
/// a term nobody recognises — `frm:ada`, a stray colon, an unparseable date — is searched for as
/// text rather than refused. The cost of that choice is a search that finds nothing rather than
/// one that explains itself, which is the right way round while the user is still typing.
///
/// `label` resolves a name to the labels that bear it; [`named`] builds one from
/// [`known_labels`]. There is deliberately no convenience wrapper that passes an empty resolver.
/// One existed, the window used it, and `label:travel` there quietly became a full-text search
/// for the literal string while the same query worked in the terminal. A caller that has no
/// index has to write `&|_| Vec::new()` and see itself do it.
///
/// `label:` needs the store and the rest of this does not, so the lookup arrives as a function
/// rather than a connection: the parser stays pure and testable, and the one term that needs the
/// world gets it from the caller. A name nothing knows becomes text — the same rule as any other
/// unrecognised term, and the same reason: a search box must not refuse what is being typed.
///
/// A *list* rather than one id, because `UNIQUE (account, name)` means the same word can name a
/// label on each account, and someone who types `label:travel` means the word.
pub fn parse_with<Tz: TimeZone>(
    input: &str,
    zone: &Tz,
    label: &dyn Fn(&str) -> Vec<LabelId>,
) -> Filter {
    let mut clauses: Vec<Filter> = Vec::new();
    let mut words: Vec<String> = Vec::new();

    for token in tokenize(input) {
        match term(&token, zone, label) {
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
pub struct Token {
    /// The token without its `-` and its quotes: `from:ada`, `lunch on friday`.
    pub text: String,
    /// Led by `-`.
    pub negated: bool,
    /// Quoted, whole (`"lunch on friday"`) or as a field's value (`subject:"lunch"`).
    pub quoted: bool,
}

/// `input` as the search reads it, one token at a time: for a surface that shows a query as
/// parts (a rule's conditions) and must split it as [`parse_with`] does.
pub fn tokens(input: &str) -> Vec<Token> {
    tokenize(input)
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
fn term<Tz: TimeZone>(
    token: &Token,
    zone: &Tz,
    label: &dyn Fn(&str) -> Vec<LabelId>,
) -> Option<Filter> {
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
            // Starred or pinned. The one `Or` a term stands for.
            "kept" => crate::view::kept_filter(),
            _ => return None,
        },
        "in" => Filter::InMailbox(mailbox(value)?),
        // A name that is not one of Gmail's tabs is text, like any unknown value.
        "category" => Filter::Category(Category::named(value)?),
        "label" => match label(value).as_slice() {
            // Nothing bears that name: the term becomes text, like any other unknown one.
            [] => return None,
            [one] => Filter::HasLabel(*one),
            // The same word on more than one account. `Or`, because the user meant the word.
            many => Filter::Or(many.iter().map(|id| Filter::HasLabel(*id)).collect()),
        },
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
    // A value that names no mailbox is not a mailbox filter.
    value.parse().ok()
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

#[cfg(test)]
mod category_tests {
    use super::*;

    fn parsed(input: &str) -> Filter {
        parse_with(input, &Utc, &|_| Vec::new())
    }

    #[test]
    fn category_terms() {
        let text = |t: &str| Filter::Text(TextMatch::Contains(t.to_owned()));
        let cases: Vec<(&str, Filter)> = vec![
            ("category:primary", Filter::Category(Category::Primary)),
            ("category:social", Filter::Category(Category::Social)),
            (
                "category:promotions",
                Filter::Category(Category::Promotions),
            ),
            ("category:updates", Filter::Category(Category::Updates)),
            ("category:forums", Filter::Category(Category::Forums)),
            (
                "CATEGORY:Promotions",
                Filter::Category(Category::Promotions),
            ),
            (
                "-category:social",
                Filter::Not(Box::new(Filter::Category(Category::Social))),
            ),
            (
                "from:ada category:updates",
                Filter::And(vec![
                    Filter::From(TextMatch::Contains("ada".to_owned())),
                    Filter::Category(Category::Updates),
                ]),
            ),
            // Not one of the tabs, or nothing after the colon: searched for as text.
            ("category:spam", text("category:spam")),
            ("category:", text("category:")),
        ];
        for (input, want) in cases {
            assert_eq!(parsed(input), want, "{input}");
        }
    }
}
