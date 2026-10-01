//! A search as IMAP `UID SEARCH` (RFC 3501 §6.4.4, RFC 9051 §6.4.4), one mailbox at a time.
//!
//! IMAP searches the selected mailbox, so where to search is half the query. A query that names a
//! place at the top (`in:sent`, a folder) searches there. Otherwise Gmail's All Mail — the mailbox
//! the server marks `\All` (RFC 6154) — holds every message and is searched alone; elsewhere the
//! inbox and the archive are.
//!
//! Each clause becomes the key that searches the same field: `from:` is `FROM`, `to:` is `TO` or
//! `CC` (the store's `to:` is both), a free word is found in the subject, a sender, a recipient or
//! the body, as the store's index holds exactly those. There is no key for "has an attachment", and
//! pinned and snoozed are this client's own; those are [`Unsaid`].
//!
//! Strings go as quoted strings when they are printable ASCII, and as literals otherwise, with
//! `CHARSET UTF-8` in front (RFC 3501 §6.4.4; IMAP4rev2 servers must accept it, RFC 9051 §6.4.4).
//! Where the server has `ESEARCH` (RFC 4731) the answer is asked for as `RETURN (COUNT ALL)`: a
//! count and a compact set, so ten thousand matches are a line, not ten thousand numbers.

use super::{
    Asked, Place, Unsaid, describe, each, imap_date, kept_here, on_account, place_of, whole_days,
    words,
};
use crate::imap::{Untagged, has_capability};
use crate::machine::ProtoError;
use chrono::NaiveDate;
use mail_domain::{
    AccountId, Filter, Folder, FolderRoles, LabelId, MailboxRole, ReadState, ServerLabels,
    SpecialUse, Star, TextMatch,
};

/// One search key (RFC 3501 §6.4.4), as a tree rather than text so a test can read it and nothing
/// but these reaches the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchKey {
    All,
    Seen,
    Unseen,
    Flagged,
    Unflagged,
    From(String),
    To(String),
    Cc(String),
    Subject(String),
    Body(String),
    /// `SENTSINCE`: the `Date` header's day is this one or later.
    SentSince(NaiveDate),
    /// `SENTBEFORE`: the `Date` header's day is before this one.
    SentBefore(NaiveDate),
    /// Gmail's `X-GM-LABELS` (its IMAP extensions): the message bears this label.
    GmailLabel(String),
    Not(Box<SearchKey>),
    Or(Box<SearchKey>, Box<SearchKey>),
    /// Every key, parenthesised where it is not the whole search.
    And(Vec<SearchKey>),
}

/// What an IMAP account is searched with: where, and for what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapPlan {
    /// The mailboxes to search, each as the server names it.
    pub mailboxes: Vec<String>,
    /// The keys, all of which a match satisfies.
    pub keys: Vec<SearchKey>,
}

/// What the translation needs to know about the account.
#[derive(Debug, Clone, Copy)]
pub struct ImapCtx<'a> {
    pub account: AccountId,
    /// The folders the server lists, for the one marked `\All`.
    pub folders: &'a [Folder],
    pub roles: &'a FolderRoles,
    pub labels: ServerLabels,
    /// Each label this client knows on the account, with its name as the server spells it.
    pub labels_named: &'a [(LabelId, String)],
}

/// `filter` as a search of this IMAP account, or what of it cannot be asked.
pub fn translate(filter: &Filter, ctx: &ImapCtx<'_>) -> Result<Asked<ImapPlan>, Unsaid> {
    let filter = on_account(filter, ctx.account);
    if filter == Filter::Nothing {
        return Ok(Asked::Nothing);
    }
    let (place, rest) = place_of(&filter)?;
    let keyed = each(&rest, |clause| key(clause, ctx));
    let (mailboxes, extra) = match place {
        Some(Place::Folder(path)) => (vec![path], Vec::new()),
        Some(Place::Role(role)) => where_role(role, ctx)?,
        None => (everywhere(ctx), Vec::new()),
    };
    let mut keys = keyed?;
    keys.extend(extra);
    if keys.is_empty() {
        keys.push(SearchKey::All);
    }
    Ok(Asked::Ask(ImapPlan { mailboxes, keys }))
}

/// The mailbox `in:<role>` searches, and any key it needs beside it.
fn where_role(
    role: MailboxRole,
    ctx: &ImapCtx<'_>,
) -> Result<(Vec<String>, Vec<SearchKey>), Unsaid> {
    let path = match (role, ctx.roles.path(role)) {
        (_, Some(path)) => path.to_owned(),
        (MailboxRole::Inbox, None) => "INBOX".to_owned(),
        (_, None) => {
            return Err(Unsaid::one(format!(
                "in:{} (the server names no such folder)",
                super::role_word(role)
            )));
        }
    };
    // On Gmail the archive is All Mail less the inbox: its folder holds the inbox's mail too.
    let archive_is_all = role == MailboxRole::Archive
        && ctx.labels == ServerLabels::Supported
        && ctx
            .folders
            .iter()
            .any(|f| f.path == path && f.special == Some(SpecialUse::All));
    let extra = if archive_is_all {
        vec![SearchKey::Not(Box::new(SearchKey::GmailLabel(
            "\\Inbox".to_owned(),
        )))]
    } else {
        Vec::new()
    };
    Ok((vec![path], extra))
}

/// Where a query that names no place is searched: All Mail where the server has one, else the
/// inbox and the archive.
fn everywhere(ctx: &ImapCtx<'_>) -> Vec<String> {
    if let Some(all) = ctx
        .folders
        .iter()
        .find(|f| f.special == Some(SpecialUse::All))
    {
        return vec![all.path.clone()];
    }
    let inbox = ctx
        .roles
        .path(MailboxRole::Inbox)
        .unwrap_or("INBOX")
        .to_owned();
    let mut out = vec![inbox.clone()];
    if let Some(archive) = ctx.roles.path(MailboxRole::Archive)
        && archive != inbox
    {
        out.push(archive.to_owned());
    }
    out
}

/// A word found wherever the store's index would find it: subject, sender, recipients, body.
fn anywhere(word: &str) -> SearchKey {
    let fields = [
        SearchKey::Subject(word.to_owned()),
        SearchKey::From(word.to_owned()),
        SearchKey::To(word.to_owned()),
        SearchKey::Cc(word.to_owned()),
        SearchKey::Body(word.to_owned()),
    ];
    either(fields.into_iter().collect())
}

/// `OR` over every key, nested to the right.
fn either(mut keys: Vec<SearchKey>) -> SearchKey {
    let last = keys.pop().unwrap_or(SearchKey::All);
    keys.into_iter().rev().fold(last, |rest, key| {
        SearchKey::Or(Box::new(key), Box::new(rest))
    })
}

fn both(keys: Vec<SearchKey>) -> SearchKey {
    match <[SearchKey; 1]>::try_from(keys) {
        Ok([one]) => one,
        Err(keys) => SearchKey::And(keys),
    }
}

fn key(filter: &Filter, ctx: &ImapCtx<'_>) -> Result<SearchKey, Unsaid> {
    let whole = |m: &TextMatch| match m {
        TextMatch::Contains(s) => Ok(s.clone()),
        // A whole value is not something IMAP compares; its keys find substrings.
        TextMatch::Exact(_) => Err(Unsaid::one(format!(
            "{} as a whole value",
            describe(filter)
        ))),
    };
    Ok(match filter {
        Filter::All => SearchKey::All,
        Filter::Nothing => SearchKey::Not(Box::new(SearchKey::All)),
        Filter::And(parts) => both(each(parts, |p| key(p, ctx))?),
        Filter::Or(parts) => either(each(parts, |p| key(p, ctx))?),
        Filter::Not(inner) => SearchKey::Not(Box::new(key(inner, ctx)?)),
        Filter::Read(ReadState::Read) => SearchKey::Seen,
        Filter::Read(ReadState::Unread) => SearchKey::Unseen,
        Filter::Starred(Star::Starred) => SearchKey::Flagged,
        Filter::Starred(Star::Unstarred) => SearchKey::Unflagged,
        Filter::From(m) => SearchKey::From(whole(m)?),
        Filter::To(m) => {
            let who = whole(m)?;
            SearchKey::Or(
                Box::new(SearchKey::To(who.clone())),
                Box::new(SearchKey::Cc(who)),
            )
        }
        Filter::Subject(m) => SearchKey::Subject(whole(m)?),
        Filter::Text(TextMatch::Contains(needle)) => {
            both(words(needle).iter().map(|w| anywhere(w)).collect())
        }
        // A phrase is one string, found whole in one field.
        Filter::Text(TextMatch::Exact(phrase)) => anywhere(phrase),
        Filter::Date(range) => {
            let (since, before) = whole_days(range);
            let mut keys = Vec::new();
            keys.extend(since.map(SearchKey::SentSince));
            keys.extend(before.map(SearchKey::SentBefore));
            if keys.is_empty() {
                SearchKey::All
            } else {
                both(keys)
            }
        }
        Filter::HasLabel(label) => match (ctx.labels, super::named(ctx.labels_named, *label)) {
            (ServerLabels::Supported, Some(name)) => SearchKey::GmailLabel(name.to_owned()),
            _ => {
                return Err(Unsaid::one(
                    "label: (the server keeps no labels to search)".to_owned(),
                ));
            }
        },
        Filter::HasAttachment => {
            return Err(Unsaid::one(
                "has:attachment (IMAP has no search for it)".to_owned(),
            ));
        }
        // A place under OR or NOT: a mailbox is where IMAP searches, not what it matches.
        Filter::InMailbox(_) | Filter::InFolder(_) => {
            return Err(Unsaid::one(format!(
                "{} inside another clause",
                describe(filter)
            )));
        }
        Filter::Account(_) | Filter::Snoozed | Filter::SnoozeDue | Filter::Pinned => {
            return Err(kept_here(filter));
        }
    })
}

/// One piece of a command line: text, or a string that has to go as a literal.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Literal(Vec<u8>),
}

/// `keys` as the command `UID SEARCH`, split where the server must say `+` before a literal is
/// sent (RFC 3501 §7.5). One part when no literal needs asking for; each later part is what goes
/// after a continuation. The last ends the line.
///
/// `caps` is what the server has said it supports on this connection: `ESEARCH` for the
/// compact answer, `LITERAL+` or `LITERAL-` for literals that need no `+` (RFC 7888).
pub(crate) fn command(
    tag: &str,
    keys: &[SearchKey],
    caps: &[String],
) -> Result<Vec<Vec<u8>>, ProtoError> {
    let mut pieces = vec![Piece::Text(format!("{tag} UID SEARCH"))];
    if has_capability(caps, "ESEARCH") {
        pieces.push(Piece::Text(" RETURN (COUNT ALL)".to_owned()));
    }
    if keys.iter().any(needs_utf8) {
        pieces.push(Piece::Text(" CHARSET UTF-8".to_owned()));
    }
    for key in keys {
        pieces.push(Piece::Text(" ".to_owned()));
        render(key, &mut pieces)?;
    }
    let non_sync = |len: usize| {
        has_capability(caps, "LITERAL+") || (has_capability(caps, "LITERAL-") && len <= 4096)
    };
    let mut parts = vec![Vec::new()];
    for piece in pieces {
        let current = parts.last_mut().expect("never empty");
        match piece {
            Piece::Text(text) => current.extend_from_slice(text.as_bytes()),
            Piece::Literal(bytes) if non_sync(bytes.len()) => {
                current.extend_from_slice(format!("{{{}+}}\r\n", bytes.len()).as_bytes());
                current.extend_from_slice(&bytes);
            }
            Piece::Literal(bytes) => {
                current.extend_from_slice(format!("{{{}}}\r\n", bytes.len()).as_bytes());
                parts.push(bytes);
            }
        }
    }
    parts
        .last_mut()
        .expect("never empty")
        .extend_from_slice(b"\r\n");
    Ok(parts)
}

fn needs_utf8(key: &SearchKey) -> bool {
    match key {
        SearchKey::From(s)
        | SearchKey::To(s)
        | SearchKey::Cc(s)
        | SearchKey::Subject(s)
        | SearchKey::Body(s)
        | SearchKey::GmailLabel(s) => !s.is_ascii(),
        SearchKey::Not(inner) => needs_utf8(inner),
        SearchKey::Or(a, b) => needs_utf8(a) || needs_utf8(b),
        SearchKey::And(keys) => keys.iter().any(needs_utf8),
        _ => false,
    }
}

fn render(key: &SearchKey, out: &mut Vec<Piece>) -> Result<(), ProtoError> {
    let word = |out: &mut Vec<Piece>, w: &str| out.push(Piece::Text(w.to_owned()));
    let field = |out: &mut Vec<Piece>, name: &str, value: &str| -> Result<(), ProtoError> {
        out.push(Piece::Text(format!("{name} ")));
        out.push(string(value)?);
        Ok(())
    };
    match key {
        SearchKey::All => word(out, "ALL"),
        SearchKey::Seen => word(out, "SEEN"),
        SearchKey::Unseen => word(out, "UNSEEN"),
        SearchKey::Flagged => word(out, "FLAGGED"),
        SearchKey::Unflagged => word(out, "UNFLAGGED"),
        SearchKey::From(v) => field(out, "FROM", v)?,
        SearchKey::To(v) => field(out, "TO", v)?,
        SearchKey::Cc(v) => field(out, "CC", v)?,
        SearchKey::Subject(v) => field(out, "SUBJECT", v)?,
        SearchKey::Body(v) => field(out, "BODY", v)?,
        SearchKey::GmailLabel(v) => field(out, "X-GM-LABELS", v)?,
        SearchKey::SentSince(d) => word(out, &format!("SENTSINCE {}", imap_date(*d))),
        SearchKey::SentBefore(d) => word(out, &format!("SENTBEFORE {}", imap_date(*d))),
        SearchKey::Not(inner) => {
            word(out, "NOT ");
            render(inner, out)?;
        }
        SearchKey::Or(a, b) => {
            word(out, "OR ");
            render(a, out)?;
            word(out, " ");
            render(b, out)?;
        }
        SearchKey::And(keys) => {
            word(out, "(");
            for (n, key) in keys.iter().enumerate() {
                if n > 0 {
                    word(out, " ");
                }
                render(key, out)?;
            }
            word(out, ")");
        }
    }
    Ok(())
}

/// A search string: quoted where it is printable ASCII, a literal where it is not.
///
/// A line break cannot be searched for and would end the command, so a string holding one is
/// refused rather than sent.
fn string(value: &str) -> Result<Piece, ProtoError> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0)) {
        return Err(ProtoError::Malformed(
            "a search string holds a line break".to_owned(),
        ));
    }
    if value.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        let mut out = String::with_capacity(value.len() + 2);
        out.push('"');
        for ch in value.chars() {
            if ch == '\\' || ch == '"' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
        return Ok(Piece::Text(out));
    }
    Ok(Piece::Literal(value.as_bytes().to_vec()))
}

/// What one `UID SEARCH` found: every UID, and how many the server counted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hits {
    /// Ascending, as both answers give them.
    pub uids: Vec<u32>,
    /// The server's count where it gave one (`ESEARCH`'s `COUNT`), else how many UIDs came.
    pub count: u64,
}

/// The answer to the search whose responses arrived during command `during`: an `ESEARCH`
/// (RFC 4731 §3.1) or a `SEARCH` (RFC 3501 §7.2.5), whichever the server sent.
pub(crate) fn hits(untagged: &[Untagged], during: usize) -> Result<Hits, ProtoError> {
    let mut uids = Vec::new();
    let mut count = None;
    for response in untagged.iter().filter(|u| u.during == during) {
        let text = response.text.trim();
        let Some(rest) = text.strip_prefix("* ") else {
            continue;
        };
        let (name, rest) = rest.split_once(' ').unwrap_or((rest, ""));
        if name.eq_ignore_ascii_case("SEARCH") {
            for n in rest.split_whitespace() {
                // `* SEARCH 2 5 (MODSEQ 917)` under CONDSTORE: the modseq is not a UID.
                if n.starts_with('(') {
                    break;
                }
                uids.push(uid(n)?);
            }
        } else if name.eq_ignore_ascii_case("ESEARCH") {
            let (found, counted) = esearch(rest)?;
            uids.extend(found);
            count = count.or(counted);
        }
    }
    uids.sort_unstable();
    uids.dedup();
    let count = count.unwrap_or(uids.len() as u64);
    Ok(Hits { uids, count })
}

fn uid(text: &str) -> Result<u32, ProtoError> {
    text.parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| ProtoError::Malformed(format!("{text:?} is not a UID in a search answer")))
}

/// `(TAG "a4") UID COUNT 3 ALL 4:5,9`: the UIDs of `ALL`, and `COUNT`.
fn esearch(rest: &str) -> Result<(Vec<u32>, Option<u64>), ProtoError> {
    let mut rest = rest.trim();
    // The correlator names the command; one search is outstanding at a time, so it is skipped.
    if rest.starts_with('(') {
        let end = rest.find(')').ok_or_else(|| {
            ProtoError::Malformed("an ESEARCH correlator is not closed".to_owned())
        })?;
        rest = rest[end + 1..].trim();
    }
    let mut words = rest.split_whitespace().peekable();
    if words.peek().is_some_and(|w| w.eq_ignore_ascii_case("UID")) {
        words.next();
    }
    let mut uids = Vec::new();
    let mut count = None;
    while let Some(name) = words.next() {
        let value = words
            .next()
            .ok_or_else(|| ProtoError::Malformed(format!("ESEARCH {name} has no value")))?;
        if name.eq_ignore_ascii_case("ALL") {
            uids.extend(set(value)?);
        } else if name.eq_ignore_ascii_case("COUNT") {
            count = Some(value.parse().map_err(|_| {
                ProtoError::Malformed(format!("ESEARCH COUNT {value:?} is not a number"))
            })?);
        }
        // MIN, MAX and anything newer carry one value each; neither was asked for.
    }
    Ok((uids, count))
}

/// Most UIDs one compact set may stand for. A set is the server's; a range to four billion
/// would otherwise be a four-billion-entry list.
const MOST_IN_A_SET: u64 = 1_000_000;

/// A sequence set of UIDs, `4:5,9`, expanded.
fn set(text: &str) -> Result<Vec<u32>, ProtoError> {
    let mut out = Vec::new();
    for part in text.split(',') {
        let (low, high) = match part.split_once(':') {
            Some((a, b)) => (uid(a)?, uid(b)?),
            None => (uid(part)?, uid(part)?),
        };
        let (low, high) = (low.min(high), low.max(high));
        if out.len() as u64 + u64::from(high - low) + 1 > MOST_IN_A_SET {
            return Err(ProtoError::Malformed(
                "a search answer names more UIDs than a mailbox holds".to_owned(),
            ));
        }
        out.extend(low..=high);
    }
    Ok(out)
}
