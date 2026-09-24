//! The rules both stores run: what a message says about whom, how an entry changes when it
//! hears it, how entries rank and which ones a typed prefix finds. Pure, so the two stores
//! cannot disagree about any of it — they differ only in where the rows live.

use super::{Contact, Kind, Origin, Tally};
use chrono::{DateTime, Utc};
use mail_domain::filter::search_tokens;
use mail_domain::{AccountId, Address, MailboxRole};
use std::cmp::Ordering;

/// How much writing to someone counts against receiving from them.
///
/// Ten: someone the user wrote to once a month outranks a sender whose mail they receive most
/// days, which is what the person typing a recipient means.
pub const WRITTEN_WEIGHT: f64 = 10.0;
/// See [`WRITTEN_WEIGHT`].
pub const RECEIVED_WEIGHT: f64 = 1.0;
/// Days after which an event counts half as much.
pub const HALF_LIFE_DAYS: f64 = 90.0;

/// An address as the book keys it: trimmed, unbracketed, lower-cased. `None` for anything
/// without a local part and a domain.
///
/// Lower-cased whole, although RFC 5321 lets a server treat the local part's case as
/// significant: none that matters does, and `Ada@x` and `ada@x` as two contacts would be a
/// duplicate in every list the user sees.
pub fn normalise(address: &str) -> Option<String> {
    let trimmed = address
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    let (local, domain) = trimmed.rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() || trimmed.contains(char::is_whitespace) {
        return None;
    }
    Some(trimmed.to_lowercase())
}

/// A display name worth keeping: trimmed, unquoted, with runs of whitespace collapsed. Nothing
/// when it is empty or merely repeats an address.
pub fn clean_name(name: Option<&str>) -> Option<String> {
    let collapsed = name?
        .trim()
        .trim_matches(['"', '\''])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() || normalise(&collapsed).is_some() {
        return None;
    }
    Some(collapsed)
}

/// Whether an address's local part says nobody reads what is sent to it: `noreply`,
/// `no-reply`, `no_reply`, `do-not-reply`, `donotreply`, and the same followed by more words
/// (`noreply-billing`), or a bounce sender. Compared as words split on the separators people use
/// in local parts — never as a substring, or `snoreply@` and `reno.reply@` would be caught.
pub fn no_reply(address: &str) -> bool {
    let local = address.split('@').next().unwrap_or_default().to_lowercase();
    let words: Vec<&str> = local
        .split(['-', '_', '.', '+'])
        .filter(|w| !w.is_empty())
        .collect();
    const PREFIXES: &[&[&str]] = &[
        &["noreply"],
        &["no", "reply"],
        &["donotreply"],
        &["do", "not", "reply"],
        &["mailer", "daemon"],
    ];
    PREFIXES.iter().any(|prefix| words.starts_with(prefix))
}

/// Whether a message's header section says it came through a list: a `List-Id` (RFC 2919), or
/// `Precedence: bulk` or `list`. `head` is the start of the raw message; only its header section
/// is read, and header names are compared whole.
pub fn listed(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head);
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("List-Id") {
            return true;
        }
        if name.eq_ignore_ascii_case("Precedence")
            && ["bulk", "list"]
                .iter()
                .any(|p| value.trim().eq_ignore_ascii_case(p))
        {
            return true;
        }
    }
    false
}

/// The parts of a message the book learns from.
#[derive(Debug, Clone, Copy)]
pub struct Seen<'a> {
    pub mailbox: MailboxRole,
    pub date: DateTime<Utc>,
    pub from: &'a Address,
    pub to: &'a [Address],
    pub cc: &'a [Address],
    pub bcc: &'a [Address],
    pub subject: &'a str,
    /// [`Kind::Bulk`] when the headers said it came through a list, else [`Kind::Person`].
    pub sender: Kind,
}

/// One thing the book hears about one address.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Received {
        at: DateTime<Utc>,
        name: Option<String>,
        /// [`Kind::Bulk`] or [`Kind::Person`], as the message looked.
        looks: Kind,
    },
    Written {
        at: DateTime<Utc>,
        name: Option<String>,
    },
    /// The address is one the user sends as.
    Own,
}

/// What a message in the store says, address by address. Spam and drafts say nothing: one is
/// not correspondence and the other has not happened yet.
pub fn events(seen: &Seen<'_>) -> Vec<(String, Event)> {
    match seen.mailbox {
        MailboxRole::Spam | MailboxRole::Drafts => Vec::new(),
        MailboxRole::Sent => sent(seen.from, [seen.to, seen.cc, seen.bcc], seen.date),
        MailboxRole::Inbox | MailboxRole::Archive | MailboxRole::Trash => {
            let Some(address) = normalise(&seen.from.email) else {
                return Vec::new();
            };
            let looks = if seen.sender == Kind::Bulk || no_reply(&address) {
                Kind::Bulk
            } else {
                Kind::Person
            };
            vec![(
                address,
                Event::Received {
                    at: seen.date,
                    name: clean_name(seen.from.name.as_deref()),
                    looks,
                },
            )]
        }
    }
}

/// What sending a message says: the sender is the user, and each recipient was written to,
/// under the name the user wrote them with. Writing to oneself is not writing to a contact.
pub fn sent(from: &Address, lists: [&[Address]; 3], at: DateTime<Utc>) -> Vec<(String, Event)> {
    let own = normalise(&from.email);
    let mut out: Vec<(String, Event)> = own.iter().map(|a| (a.clone(), Event::Own)).collect();
    for recipient in lists.iter().flat_map(|l| l.iter()) {
        let Some(address) = normalise(&recipient.email) else {
            continue;
        };
        if Some(&address) == own.as_ref() || out.iter().any(|(a, _)| *a == address) {
            continue;
        }
        out.push((
            address,
            Event::Written {
                at,
                name: clean_name(recipient.name.as_deref()),
            },
        ));
    }
    out
}

/// What identifies one sent message across its two sightings — the submission the outbox
/// confirmed and the copy that later arrives from the Sent folder — so it is counted once.
///
/// The visible recipients and the subject: both copies are built from the same draft, and
/// neither carries anything else they are certain to share. Not `Bcc`, which a Sent copy may or
/// may not keep. Two different messages with the same
/// recipients and subject would be taken for one only if the second arrived in Sent before the
/// first's copy did, which costs one count.
pub fn fingerprint(lists: [&[Address]; 2], subject: &str) -> String {
    let mut addresses: Vec<String> = lists
        .iter()
        .flat_map(|l| l.iter())
        .filter_map(|a| normalise(&a.email))
        .collect();
    addresses.sort();
    addresses.dedup();
    format!("{}\u{1f}{}", addresses.join(","), subject.trim())
}

/// Which source a name came from; a name is replaced only by one from the same source or a
/// better one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NameRank {
    Unnamed = 0,
    Received = 1,
    Written = 2,
    /// Given by the user or an address book. History never replaces it.
    Given = 3,
}

impl NameRank {
    pub fn from_i64(n: i64) -> Self {
        match n {
            1 => NameRank::Received,
            2 => NameRank::Written,
            3 => NameRank::Given,
            _ => NameRank::Unnamed,
        }
    }
}

/// An entry as stored: the [`Contact`] and what ranking and naming need beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub contact: Contact,
    /// `log2` of the frecency sum. `None` before any event.
    pub score: Option<f64>,
    pub name_rank: NameRank,
    /// When the current name was seen, for "the most recent name wins" among equals.
    pub name_at: Option<DateTime<Utc>>,
}

impl Row {
    /// A new entry for `address`, before anything is known.
    pub fn new(address: &str) -> Self {
        Self {
            contact: Contact {
                address: address.to_owned(),
                name: None,
                written: Tally::default(),
                received: Tally::default(),
                account: None,
                origin: Origin::History,
                kind: Kind::Person,
            },
            score: None,
            name_rank: NameRank::Unnamed,
            name_at: None,
        }
    }
}

/// `row` after hearing `event` from `account`'s mail.
pub fn apply(mut row: Row, account: Option<AccountId>, event: &Event) -> Row {
    let latest = row.contact.written.last.max(row.contact.received.last);
    let (at, name, rank, weight) = match event {
        Event::Own => {
            row.contact.kind = Kind::Own;
            return row;
        }
        Event::Received { at, name, looks } => {
            if *looks == Kind::Bulk && row.contact.kind == Kind::Person {
                row.contact.kind = Kind::Bulk;
            }
            tally(&mut row.contact.received, *at);
            (*at, name, NameRank::Received, RECEIVED_WEIGHT)
        }
        Event::Written { at, name } => {
            tally(&mut row.contact.written, *at);
            (*at, name, NameRank::Written, WRITTEN_WEIGHT)
        }
    };
    row.score = Some(bump(row.score, weight, at));
    if account.is_some() && latest.is_none_or(|l| at >= l) {
        row.contact.account = account;
    }
    if let Some(name) = name {
        let newer = row.name_at.is_none_or(|t| at >= t);
        if rank > row.name_rank || (rank == row.name_rank && newer) {
            row.contact.name = Some(name.clone());
            row.name_rank = rank;
            row.name_at = Some(at);
        }
    }
    row
}

/// `row` after the user, or an address book, says who this is.
///
/// A name given is kept against anything mail later says. No name leaves the current one — and
/// when the entry goes back to [`Origin::History`], lets mail rename it again.
pub fn given(mut row: Row, name: Option<&str>, origin: &Origin) -> Row {
    row.contact.origin = origin.clone();
    match clean_name(name) {
        Some(name) => {
            row.contact.name = Some(name);
            row.name_rank = NameRank::Given;
            row.name_at = None;
        }
        None if *origin == Origin::History && row.name_rank == NameRank::Given => {
            row.name_rank = NameRank::Unnamed;
        }
        None => {}
    }
    row
}

fn tally(tally: &mut Tally, at: DateTime<Utc>) {
    tally.count = tally.count.saturating_add(1);
    tally.last = tally.last.max(Some(at));
}

/// `log2(2^score + weight · 2^(t / half-life))`, without leaving the logarithm: the sum itself
/// overflows a float within a few centuries of the epoch at this half-life.
pub fn bump(score: Option<f64>, weight: f64, at: DateTime<Utc>) -> f64 {
    let days = at.timestamp() as f64 / 86_400.0;
    let term = weight.log2() + days / HALF_LIFE_DAYS;
    match score {
        None => term,
        Some(score) => {
            let (hi, lo) = if score >= term {
                (score, term)
            } else {
                (term, score)
            };
            hi + (lo - hi).exp2().ln_1p() / std::f64::consts::LN_2
        }
    }
}

/// Best first: higher score, entries never heard from last, then by address.
pub fn rank(a: &Row, b: &Row) -> Ordering {
    match (a.score, b.score) {
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| a.contact.address.cmp(&b.contact.address))
}

/// The words a prefix may match, space-separated with a space at each end so a match at the
/// start of any word is `instr(keys, ' ' || word)`: the whole address, its local part and its
/// domain as written, and the folded words of the local part and the name.
pub fn keys(contact: &Contact) -> String {
    let (local, domain) = contact
        .address
        .rsplit_once('@')
        .unwrap_or((contact.address.as_str(), ""));
    let mut words = vec![contact.address.clone(), local.to_owned(), domain.to_owned()];
    words.extend(search_tokens(local));
    words.extend(contact.name.iter().flat_map(|n| search_tokens(n)));
    let mut seen = Vec::new();
    for word in words {
        if !word.is_empty() && !seen.contains(&word) {
            seen.push(word);
        }
    }
    format!(" {} ", seen.join(" "))
}

/// What someone typed into a recipient field, ready to match against [`keys`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needle {
    /// Lower-cased and trimmed, for a prefix of an address, local part or domain.
    raw: String,
    /// Folded words, each of which must begin some key: `ren mü` finds "Renée Müller".
    words: Vec<String>,
}

impl Needle {
    pub fn new(typed: &str) -> Self {
        Self {
            raw: typed.trim().to_lowercase(),
            words: search_tokens(typed),
        }
    }

    /// Whether an entry with these [`keys`] matches.
    pub fn matches(&self, keys: &str) -> bool {
        if self.raw.is_empty() {
            return true;
        }
        let keys: Vec<&str> = keys.split_whitespace().collect();
        let begins = |w: &str| keys.iter().any(|k| k.starts_with(w));
        begins(&self.raw) || (!self.words.is_empty() && self.words.iter().all(|w| begins(w)))
    }

    /// Two strings such that every match has `' ' || one of them` somewhere in its keys, for a
    /// store to narrow by before [`Needle::matches`] decides. `None` matches everything.
    pub fn narrow(&self) -> Option<(String, String)> {
        if self.raw.is_empty() {
            return None;
        }
        let word = self.words.first().unwrap_or(&self.raw).clone();
        Some((self.raw.clone(), word))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + day * 86_400, 0).unwrap()
    }

    fn addr(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    #[test]
    fn addresses_are_normalised_or_refused() {
        const CASES: &[(&str, Option<&str>)] = &[
            (" Ada@Example.TEST ", Some("ada@example.test")),
            ("<bob@x.test>", Some("bob@x.test")),
            ("undisclosed-recipients:;", None),
            ("@x.test", None),
            ("a@", None),
            ("a b@x.test", None),
        ];
        for (input, expected) in CASES {
            assert_eq!(normalise(input).as_deref(), *expected, "{input:?}");
        }
    }

    #[test]
    fn a_name_that_is_empty_or_an_address_is_no_name() {
        const CASES: &[(Option<&str>, Option<&str>)] = &[
            (Some("  Ada   Lovelace "), Some("Ada Lovelace")),
            (Some("\"Bob\""), Some("Bob")),
            (Some("bob@x.test"), None),
            (Some("   "), None),
            (None, None),
        ];
        for (input, expected) in CASES {
            assert_eq!(clean_name(*input).as_deref(), *expected, "{input:?}");
        }
    }

    #[test]
    fn no_reply_addresses_are_recognised_by_whole_words() {
        const CASES: &[(&str, bool)] = &[
            ("noreply@x.test", true),
            ("no-reply@x.test", true),
            ("No_Reply@x.test", true),
            ("donotreply@x.test", true),
            ("do-not-reply@x.test", true),
            ("noreply-billing@x.test", true),
            ("MAILER-DAEMON@x.test", true),
            ("snoreply@x.test", false),
            ("reno.reply@x.test", false),
            ("reply@x.test", false),
            ("nora@x.test", false),
        ];
        for (input, expected) in CASES {
            assert_eq!(no_reply(input), *expected, "{input}");
        }
    }

    #[test]
    fn list_headers_are_found_in_the_header_section_only() {
        const CASES: &[(&str, bool)] = &[
            ("From: a@x\r\nList-Id: <l.x.test>\r\n\r\nbody", true),
            ("From: a@x\r\nlist-id:<l.x.test>\r\n\r\n", true),
            ("Precedence: bulk\r\n\r\n", true),
            ("Precedence: first-class\r\n\r\n", false),
            ("X-List-Id-Ish: 1\r\n\r\n", false),
            ("From: a@x\r\n\r\nList-Id: <in the body>\r\n", false),
            (
                "Subject: a\r\n List-Id: folded, not a header\r\n\r\n",
                false,
            ),
        ];
        for (input, expected) in CASES {
            assert_eq!(listed(input.as_bytes()), *expected, "{input:?}");
        }
    }

    #[test]
    fn a_sent_message_writes_to_each_recipient_once_and_not_to_the_sender() {
        let from = addr(Some("Me"), "me@x.test");
        let to = [addr(Some("Ada"), "Ada@x.test"), addr(None, "me@x.test")];
        let cc = [addr(Some("Ada L."), "ada@x.test")];
        let seen = Seen {
            mailbox: MailboxRole::Sent,
            date: at(0),
            from: &from,
            to: &to,
            cc: &cc,
            bcc: &[],
            subject: "hi",
            sender: Kind::Person,
        };
        assert_eq!(
            events(&seen),
            [
                ("me@x.test".to_owned(), Event::Own),
                (
                    "ada@x.test".to_owned(),
                    Event::Written {
                        at: at(0),
                        name: Some("Ada".into())
                    }
                ),
            ]
        );
    }

    #[test]
    fn spam_and_drafts_teach_nothing() {
        let from = addr(None, "a@x.test");
        for mailbox in [MailboxRole::Spam, MailboxRole::Drafts] {
            let seen = Seen {
                mailbox,
                date: at(0),
                from: &from,
                to: &[],
                cc: &[],
                bcc: &[],
                subject: "",
                sender: Kind::Person,
            };
            assert!(events(&seen).is_empty(), "{mailbox:?}");
        }
    }

    #[test]
    fn a_written_name_outranks_a_received_one_and_a_given_one_outranks_both() {
        let row = Row::new("a@x.test");
        let row = apply(
            row,
            None,
            &Event::Written {
                at: at(0),
                name: Some("Typed".into()),
            },
        );
        let row = apply(
            row,
            None,
            &Event::Received {
                at: at(5),
                name: Some("Theirs".into()),
                looks: Kind::Person,
            },
        );
        assert_eq!(row.contact.name.as_deref(), Some("Typed"));
        let row = given(row, Some("Mine"), &Origin::Manual);
        let row = apply(
            row,
            None,
            &Event::Written {
                at: at(9),
                name: Some("Later".into()),
            },
        );
        assert_eq!(row.contact.name.as_deref(), Some("Mine"));
        let row = given(row, None, &Origin::History);
        let row = apply(
            row,
            None,
            &Event::Received {
                at: at(10),
                name: Some("Theirs again".into()),
                looks: Kind::Person,
            },
        );
        assert_eq!(row.contact.name.as_deref(), Some("Theirs again"));
    }

    #[test]
    fn among_names_from_one_source_the_most_recent_wins_whatever_order_they_arrive_in() {
        let row = Row::new("a@x.test");
        let newer = Event::Received {
            at: at(9),
            name: Some("New".into()),
            looks: Kind::Person,
        };
        let older = Event::Received {
            at: at(1),
            name: Some("Old".into()),
            looks: Kind::Person,
        };
        let row = apply(apply(row, None, &newer), None, &older);
        assert_eq!(row.contact.name.as_deref(), Some("New"));
        assert_eq!(row.contact.received.count, 2);
        assert_eq!(row.contact.received.last, Some(at(9)));
    }

    #[test]
    fn writing_once_outranks_receiving_a_few_times_but_not_forever() {
        let wrote = apply(
            Row::new("w@x.test"),
            None,
            &Event::Written {
                at: at(0),
                name: None,
            },
        );
        let heard = |days: &[i64]| {
            days.iter().fold(Row::new("r@x.test"), |row, d| {
                apply(
                    row,
                    None,
                    &Event::Received {
                        at: at(*d),
                        name: None,
                        looks: Kind::Person,
                    },
                )
            })
        };
        assert_eq!(rank(&wrote, &heard(&[0, 1, 2])), Ordering::Less);
        // A year of weekly mail since: the written-to one has decayed past it.
        let weekly: Vec<i64> = (0..52).map(|w| w * 7).collect();
        assert_eq!(rank(&wrote, &heard(&weekly)), Ordering::Greater);
    }

    #[test]
    fn a_bulk_sender_is_hidden_until_written_to_and_the_user_never_is() {
        let bulk = apply(
            Row::new("news@x.test"),
            None,
            &Event::Received {
                at: at(0),
                name: None,
                looks: Kind::Bulk,
            },
        );
        assert!(!bulk.contact.offered());
        let written = apply(
            bulk.clone(),
            None,
            &Event::Written {
                at: at(1),
                name: None,
            },
        );
        assert!(written.contact.offered());
        assert!(given(bulk, None, &Origin::Manual).contact.offered());
        let own = apply(Row::new("me@x.test"), None, &Event::Own);
        assert!(!own.contact.offered());
    }

    #[test]
    fn a_prefix_matches_the_start_of_any_word_of_the_name_or_address() {
        let contact = Contact {
            name: Some("Renée O'Brien-Müller".into()),
            ..Row::new("r.obrien@mail.example.test").contact
        };
        let keys = keys(&contact);
        const CASES: &[(&str, bool)] = &[
            ("", true),
            ("ren", true),
            ("RENEE", true),
            ("mul", true),
            ("müller", true),
            ("brien", true),
            ("obr", true),
            ("r.ob", true),
            ("mail.ex", true),
            ("r.obrien@m", true),
            ("ren mül", true),
            ("ren smith", false),
            ("enée", false),
            ("example", false),
        ];
        for (typed, expected) in CASES {
            let needle = Needle::new(typed);
            assert_eq!(needle.matches(&keys), *expected, "{typed:?} in {keys:?}");
            if *expected && let Some((raw, word)) = needle.narrow() {
                assert!(
                    keys.contains(&format!(" {raw}")) || keys.contains(&format!(" {word}")),
                    "narrowing would drop {typed:?}"
                );
            }
        }
    }

    #[test]
    fn a_sent_copy_and_its_submission_share_a_fingerprint() {
        let a = [addr(Some("Ada"), "Ada@x.test")];
        let b = [addr(None, "b@x.test")];
        assert_eq!(
            fingerprint([&a, &b], "Lunch "),
            fingerprint([&b, &a], "Lunch")
        );
        assert_ne!(
            fingerprint([&a, &[]], "Lunch"),
            fingerprint([&b, &[]], "Lunch")
        );
    }
}
