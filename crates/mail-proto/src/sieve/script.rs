//! Rules and a vacation reply, written as one Sieve script (RFC 5228).
//!
//! Only what Sieve can say the same way is written. A server runs the script on delivery, with
//! the message and nothing else in front of it, so a clause about this client's own state — a
//! label, a star, a snooze — has nothing to test, and full-text search has no Sieve form that
//! matches words the way the index does. A rule using one stays with this client, which runs
//! every rule at arrival anyway; [`Compiled::local_only`] names each one and why.
//!
//! Where both run a rule, the second run finds the work done: a message filed on delivery never
//! arrives in the inbox, and one already marked read is not marked again.

use mail_domain::{
    AccountCaps, AccountId, AfterMatch, ArchiveMeans, Filter, MailboxRole, Rule, RuleAction,
    TextMatch, Vacation,
};
use std::collections::BTreeSet;
use std::fmt;

/// Where this server keeps the roles a rule can file into, by path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Places {
    pub archive: Option<String>,
    pub trash: Option<String>,
    pub spam: Option<String>,
}

impl Places {
    /// The folders an account's capabilities name. A role the server never named has none,
    /// and a rule filing into it stays local rather than filing into a guess.
    pub fn from_caps(caps: &AccountCaps) -> Places {
        let archive = match &caps.archive {
            ArchiveMeans::MoveToFolder(path) => Some(path.clone()),
            ArchiveMeans::DropInbox | ArchiveMeans::LocalOnly => {
                caps.folders.path(MailboxRole::Archive).map(str::to_owned)
            }
        };
        Places {
            archive,
            trash: caps.folders.path(MailboxRole::Trash).map(str::to_owned),
            spam: caps.folders.path(MailboxRole::Spam).map(str::to_owned),
        }
    }

    fn of(&self, role: MailboxRole) -> Option<&str> {
        match role {
            MailboxRole::Archive => self.archive.as_deref(),
            MailboxRole::Trash => self.trash.as_deref(),
            MailboxRole::Spam => self.spam.as_deref(),
            MailboxRole::Inbox | MailboxRole::Sent | MailboxRole::Drafts => None,
        }
    }
}

/// Why a rule was left to this client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmappable {
    /// A clause Sieve has no test for, named as the search language names it.
    Clause(&'static str),
    /// Labels exist in this client, not in a server's folders.
    Label,
    /// The server named no folder for this role.
    NoFolder(MailboxRole),
    /// The server's Sieve lacks this extension.
    Missing(&'static str),
}

impl fmt::Display for Unmappable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unmappable::Clause(what) => write!(f, "{what} has no Sieve test"),
            Unmappable::Label => f.write_str("a label exists only in this client"),
            Unmappable::NoFolder(role) => write!(f, "the server named no {role:?} folder"),
            Unmappable::Missing(ext) => write!(f, "the server's Sieve has no \"{ext}\""),
        }
    }
}

/// What became of the vacation reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VacationPlaced {
    /// None was asked for.
    Absent,
    /// In the script, with its dates tested by the server (`currentdate`, RFC 5260).
    Dated,
    /// In the script without its dates, because the server cannot test them. It stays until a
    /// push after its end takes it out again.
    Undated,
    /// Left out: outside its dates, and the server cannot test them. A push inside them puts it
    /// in.
    Outside,
    /// Left out: the server's Sieve has no `vacation`.
    Unsupported,
}

/// A script, and what it could not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub script: String,
    /// Rules written into the script.
    pub mapped: Vec<String>,
    /// Enabled rules left to this client, by name, with why.
    pub local_only: Vec<(String, Unmappable)>,
    pub vacation: VacationPlaced,
}

impl Compiled {
    /// Whether the script does nothing at all: no rule and no reply in it.
    pub fn is_empty(&self) -> bool {
        self.mapped.is_empty()
            && !matches!(
                self.vacation,
                VacationPlaced::Dated | VacationPlaced::Undated
            )
    }
}

/// Write `rules` and `vacation` as one script for a server offering `extensions` (its `SIEVE`
/// capability, as tokens).
///
/// Rules run in their order, disabled ones are left out, and the vacation reply comes last, so
/// a rule that stops also keeps the reply from going to what it stopped. `now` decides whether a
/// dated reply goes in when the server cannot test dates itself.
///
/// Line endings are CRLF throughout, strings included, as RFC 5228's grammar has them.
pub fn compile(
    rules: &[Rule],
    vacation: Option<&Vacation>,
    extensions: &[String],
    places: &Places,
    now: chrono::DateTime<chrono::Utc>,
) -> Compiled {
    let has = |name: &str| extensions.iter().any(|e| e.eq_ignore_ascii_case(name));
    let ordered = mail_domain::rule::ordered(rules);

    let mut require: BTreeSet<&'static str> = BTreeSet::new();
    let mut body = String::new();
    let mut mapped = Vec::new();
    let mut local_only = Vec::new();
    for rule in ordered {
        match block(rule, places, &has) {
            Ok((text, needs)) => {
                require.extend(needs);
                body.push_str(&text);
                mapped.push(rule.name.clone());
            }
            Err(why) => local_only.push((rule.name.clone(), why)),
        }
    }

    let placed = match vacation {
        None => VacationPlaced::Absent,
        Some(_) if !has("vacation") => VacationPlaced::Unsupported,
        Some(v) => {
            let dated = has("date") && has("relational");
            let bounded = v.during.from.is_some() || v.during.to.is_some();
            if bounded && !dated && !v.active_at(now) {
                VacationPlaced::Outside
            } else {
                require.insert("vacation");
                if bounded && dated {
                    require.insert("date");
                    require.insert("relational");
                }
                body.push_str(&vacation_block(v, bounded && dated));
                if bounded && dated {
                    VacationPlaced::Dated
                } else {
                    VacationPlaced::Undated
                }
            }
        }
    };

    let mut script = String::from(
        "# Written by mailo, and replaced whenever its rules change: edit them there.\r\n",
    );
    if !require.is_empty() {
        let names: Vec<String> = require.iter().map(|n| quoted(n)).collect();
        script.push_str(&format!("require [{}];\r\n", names.join(", ")));
    }
    script.push_str(&body);
    Compiled {
        script,
        mapped,
        local_only,
        vacation: placed,
    }
}

/// One rule as an `if` block, with the extensions it needs.
fn block(
    rule: &Rule,
    places: &Places,
    has: &dyn Fn(&str) -> bool,
) -> Result<(String, Vec<&'static str>), Unmappable> {
    let condition = test(&rule.filter, rule.account)?;
    let mut needs = Vec::new();
    let mut flags = Vec::new();
    // Sieve files a message once per `fileinto`, so several would leave copies in several
    // folders. The last one wins, as it does when this client runs the rule.
    let mut destination: Option<String> = None;
    for action in &rule.actions {
        match action {
            RuleAction::Label(_) => return Err(Unmappable::Label),
            RuleAction::MarkRead => flags.push("\\Seen"),
            RuleAction::Star => flags.push("\\Flagged"),
            RuleAction::Archive => destination = Some(place(places, MailboxRole::Archive)?),
            RuleAction::Trash => destination = Some(place(places, MailboxRole::Trash)?),
            RuleAction::Spam => destination = Some(place(places, MailboxRole::Spam)?),
            RuleAction::File(path) => destination = Some(path.clone()),
        }
    }
    if !flags.is_empty() {
        if !has("imap4flags") {
            return Err(Unmappable::Missing("imap4flags"));
        }
        needs.push("imap4flags");
    }
    if destination.is_some() {
        if !has("fileinto") {
            return Err(Unmappable::Missing("fileinto"));
        }
        needs.push("fileinto");
    }

    let mut out = format!("\r\n# {}\r\nif {condition} {{\r\n", comment(&rule.name));
    // Flags first: `fileinto` stores the message with the flags set so far (RFC 5232 §3).
    for flag in flags {
        out.push_str(&format!("    addflag {};\r\n", quoted(flag)));
    }
    if let Some(path) = destination {
        out.push_str(&format!("    fileinto {};\r\n", quoted(&path)));
    }
    if rule.after == AfterMatch::Stop {
        out.push_str("    stop;\r\n");
    }
    out.push_str("}\r\n");
    Ok((out, needs))
}

fn place(places: &Places, role: MailboxRole) -> Result<String, Unmappable> {
    places
        .of(role)
        .map(str::to_owned)
        .ok_or(Unmappable::NoFolder(role))
}

/// A filter as a Sieve test, or the clause that has none.
///
/// Text comparisons use Sieve's default comparator, `i;ascii-casemap`: ASCII case folded and
/// nothing else, which is what `From`, `To` and `Subject` promise here too. Header tests
/// decode RFC 2047 words before comparing (RFC 5228 §2.7.2), as this client's parser does.
fn test(filter: &Filter, account: AccountId) -> Result<String, Unmappable> {
    Ok(match filter {
        Filter::All => "true".to_owned(),
        Filter::Nothing => "false".to_owned(),
        Filter::And(fs) => joined("allof", "true", fs, account)?,
        Filter::Or(fs) => joined("anyof", "false", fs, account)?,
        Filter::Not(f) => format!("not {}", test(f, account)?),
        // The script is the account's own, so the clause is settled before it is written.
        Filter::Account(id) => if *id == account { "true" } else { "false" }.to_owned(),
        Filter::From(m) => address_test(&["from"], m, "from:")?,
        Filter::To(m) => address_test(&["to", "cc"], m, "to:")?,
        Filter::Subject(TextMatch::Contains(s)) => {
            format!("header :contains \"subject\" {}", quoted(s))
        }
        Filter::Subject(TextMatch::Exact(s)) => format!("header :is \"subject\" {}", quoted(s)),
        Filter::Text(_) => return Err(Unmappable::Clause("full text")),
        Filter::InMailbox(_) | Filter::InFolder(_) => return Err(Unmappable::Clause("in:")),
        Filter::Read(_) => return Err(Unmappable::Clause("is:read / is:unread")),
        Filter::Starred(_) => return Err(Unmappable::Clause("is:starred")),
        Filter::HasLabel(_) => return Err(Unmappable::Clause("label:")),
        Filter::Date(_) => return Err(Unmappable::Clause("before: / after:")),
        Filter::HasAttachment => return Err(Unmappable::Clause("has:attachment")),
        Filter::Snoozed | Filter::SnoozeDue => return Err(Unmappable::Clause("is:snoozed")),
        Filter::Pinned => return Err(Unmappable::Clause("is:pinned")),
    })
}

/// `allof (a, b)`, with the empty case written as the fold's identity.
fn joined(
    name: &str,
    empty: &str,
    filters: &[Filter],
    account: AccountId,
) -> Result<String, Unmappable> {
    let tests = filters
        .iter()
        .map(|f| test(f, account))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match tests.as_slice() {
        [] => empty.to_owned(),
        [one] => one.clone(),
        _ => format!("{name} ({})", tests.join(", ")),
    })
}

/// An address clause. `Contains` looks in the whole header, which holds both the name and the
/// address this client matches separately; that is the same answer unless the needle spans the
/// two, so a needle with the punctuation that sits between them stays local. `Exact` is written
/// only for a whole address: an exact display name has no Sieve test.
fn address_test(
    headers: &[&str],
    m: &TextMatch,
    clause: &'static str,
) -> Result<String, Unmappable> {
    let list = if let [one] = headers {
        quoted(one)
    } else {
        let names: Vec<String> = headers.iter().map(|h| quoted(h)).collect();
        format!("[{}]", names.join(", "))
    };
    match m {
        TextMatch::Contains(s) if !s.contains(['<', '>', '"', ',']) => {
            Ok(format!("header :contains {list} {}", quoted(s)))
        }
        TextMatch::Exact(s) if s.contains('@') => {
            Ok(format!("address :all :is {list} {}", quoted(s)))
        }
        TextMatch::Contains(_) | TextMatch::Exact(_) => Err(Unmappable::Clause(clause)),
    }
}

/// The vacation reply, tested against its dates when `dated`.
fn vacation_block(v: &Vacation, dated: bool) -> String {
    let mut command = format!(
        "vacation :days {} :subject {}",
        v.days.max(1),
        quoted(&v.subject)
    );
    if let Some(from) = &v.from {
        command.push_str(&format!(" :from {}", quoted(from)));
    }
    if !v.addresses.is_empty() {
        let list: Vec<String> = v.addresses.iter().map(|a| quoted(a)).collect();
        command.push_str(&format!(" :addresses [{}]", list.join(", ")));
    }
    command.push_str(&format!(" {};", quoted(&v.body)));

    let mut out = String::from("\r\n# Vacation reply\r\n");
    let mut bounds = Vec::new();
    if dated {
        // Compared as ISO 8601 text in UTC, to the second and without a zone suffix: a string
        // with the server's suffix sorts after the same instant without one, so "at or after the
        // start" holds from the start's first second and "before the end" stops at the end's.
        if let Some(from) = v.during.from {
            bounds.push(format!(
                "currentdate :zone \"+0000\" :value \"ge\" \"iso8601\" {}",
                quoted(&from.format("%Y-%m-%dT%H:%M:%S").to_string())
            ));
        }
        if let Some(to) = v.during.to {
            bounds.push(format!(
                "currentdate :zone \"+0000\" :value \"lt\" \"iso8601\" {}",
                quoted(&to.format("%Y-%m-%dT%H:%M:%S").to_string())
            ));
        }
    }
    match bounds.as_slice() {
        [] => out.push_str(&format!("{command}\r\n")),
        [one] => out.push_str(&format!("if {one} {{\r\n    {command}\r\n}}\r\n")),
        _ => out.push_str(&format!(
            "if allof ({}) {{\r\n    {command}\r\n}}\r\n",
            bounds.join(", ")
        )),
    }
    out
}

/// `text` as a Sieve quoted string (RFC 5228 §2.4.2).
///
/// A backslash and a double quote are escaped, line breaks become CRLF (a quoted string may
/// span lines), and NUL, which no Sieve string may hold, is dropped.
pub fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0' => {}
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\r\n");
            }
            '\n' => out.push_str("\r\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// A rule's name as a comment: on one line, so a name cannot end the comment and start a
/// command.
fn comment(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
