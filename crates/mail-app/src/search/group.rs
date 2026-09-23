//! Top hit, Mail, People, Actions.
//!
//! The top hit is the single best item across the groups, and it is removed from the group it
//! came from so the menu does not show it twice. Mail keeps at most six rows, people at most
//! three. An empty query is not a search: it is the actions and the five most recent threads.

use chrono::{DateTime, Utc};
use mail_domain::{Filter, ThreadSummary};

use super::RankedMail;
use super::fuzzy::{self, FuzzyHit};
use super::highlight;
use super::ranking::Affinity;
use super::source::Source;

/// Mail rows under the top hit.
const MAIL_CAP: usize = 6;
/// People rows.
const PEOPLE_CAP: usize = 3;
/// Recent threads an empty query shows, counting the top hit.
const RECENT: usize = 5;

/// Nucleo scores an exact short match near 200. Dividing by this puts that match above a
/// body-only recency bonus and below a subject bonus plus recency, so Enter opens the mail when
/// the subject matched and the command when the query is the command.
const FUZZY_PER_RANK: f64 = 100.0;

/// A command the menu can offer. `label` is what the matcher compares and what the row shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub label: String,
}

/// One mail row.
#[derive(Debug, Clone, PartialEq)]
pub struct MailHit {
    pub summary: ThreadSummary,
    /// The ranker's score. Higher is better.
    pub score: f64,
    /// Char indices of a fuzzy match in the subject. Empty when the thread is here because the
    /// body matched and the subject did not.
    pub indices: Vec<u32>,
    /// Byte ranges of the matched terms in the subject.
    pub marks: Vec<std::ops::Range<usize>>,
    /// The subject cut around the first mark, or the stored snippet when nothing was marked.
    pub preview: String,
}

/// One person, taken from the affinity map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonHit {
    pub email: String,
    pub threads: u32,
    pub replied: bool,
    pub score: u32,
    pub indices: Vec<u32>,
}

/// One action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionHit {
    pub command: Command,
    pub score: u32,
    pub indices: Vec<u32>,
}

/// The row Enter opens.
#[derive(Debug, Clone, PartialEq)]
pub enum Top {
    Mail(Box<MailHit>),
    Person(PersonHit),
    Action(ActionHit),
}

/// What the menu renders.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Results {
    pub top: Option<Top>,
    pub mail: Vec<MailHit>,
    pub people: Vec<PersonHit>,
    pub actions: Vec<ActionHit>,
}

/// Actions, in the order given, and the five most recent threads.
///
/// `top` is the newest thread. It is not repeated in `mail`, so `top` plus `mail` is those five
/// (or fewer, when the store has fewer).
pub fn empty_query(source: &dyn Source, commands: &[Command], now: DateTime<Utc>) -> Results {
    let recent = source.ranked(&Filter::All, RECENT, now);
    let mut mail: Vec<MailHit> = recent
        .into_iter()
        .map(|(summary, score)| MailHit {
            preview: summary.snippet.clone(),
            summary,
            score,
            indices: Vec::new(),
            marks: Vec::new(),
        })
        .collect();
    let top = if mail.is_empty() {
        None
    } else {
        Some(Top::Mail(Box::new(mail.remove(0))))
    };
    Results {
        top,
        mail,
        people: Vec::new(),
        actions: commands
            .iter()
            .cloned()
            .map(|command| ActionHit {
                command,
                score: 0,
                indices: Vec::new(),
            })
            .collect(),
    }
}

/// Group `ranked` into the menu.
pub fn assemble(ranked: &RankedMail, affinity: &Affinity, commands: &[Command]) -> Results {
    let query = fuzzy_query(ranked);
    let mut mail = mail_hits(ranked, &query);
    let mut people = people_hits(affinity, &query);
    let mut actions = action_hits(commands, &query);

    let top = match best_which(&mail, &people, &actions) {
        Some(Which::Mail) if !mail.is_empty() => Some(Top::Mail(Box::new(mail.remove(0)))),
        Some(Which::Person) if !people.is_empty() => Some(Top::Person(people.remove(0))),
        Some(Which::Action) if !actions.is_empty() => Some(Top::Action(actions.remove(0))),
        _ => None,
    };
    mail.truncate(MAIL_CAP);
    people.truncate(PEOPLE_CAP);
    Results {
        top,
        mail,
        people,
        actions,
    }
}

#[derive(Clone, Copy)]
enum Which {
    Mail,
    Person,
    Action,
}

/// Highest comparable score. A tie prefers mail, then a person, then an action.
fn best_which(mail: &[MailHit], people: &[PersonHit], actions: &[ActionHit]) -> Option<Which> {
    let mut chosen: Option<(f64, u8, Which)> = None;
    let candidates = [
        (mail.first().map(|hit| hit.score), 0, Which::Mail),
        (
            people
                .first()
                .map(|hit| f64::from(hit.score) / FUZZY_PER_RANK),
            1,
            Which::Person,
        ),
        (
            actions
                .first()
                .map(|hit| f64::from(hit.score) / FUZZY_PER_RANK),
            2,
            Which::Action,
        ),
    ];
    for (score, tie, which) in candidates {
        let Some(score) = score else {
            continue;
        };
        let replace = match chosen {
            None => true,
            Some((best, best_tie, _)) => score > best || (score == best && tie < best_tie),
        };
        if replace {
            chosen = Some((score, tie, which));
        }
    }
    chosen.map(|(_, _, which)| which)
}

fn mail_hits(ranked: &RankedMail, query: &str) -> Vec<MailHit> {
    let subjects: Vec<&str> = ranked
        .hits
        .iter()
        .map(|(summary, _)| summary.subject.as_str())
        .collect();
    let mut indices = vec![Vec::new(); subjects.len()];
    for hit in fuzzy::match_list(query, &subjects) {
        if let Some(slot) = indices.get_mut(hit.index) {
            *slot = hit.indices;
        }
    }
    ranked
        .hits
        .iter()
        .enumerate()
        .map(|(i, (summary, score))| {
            let marks = highlight::marks(&summary.subject, &ranked.expansion.needles);
            let preview = match marks.first() {
                Some(range) => highlight::snippet(&summary.subject, range.clone()),
                None => summary.snippet.clone(),
            };
            MailHit {
                summary: summary.clone(),
                score: *score,
                indices: indices.get(i).cloned().unwrap_or_default(),
                marks,
                preview,
            }
        })
        .collect()
}

fn people_hits(affinity: &Affinity, query: &str) -> Vec<PersonHit> {
    let emails: Vec<String> = affinity.iter().map(|(email, _)| email.to_owned()).collect();
    let refs: Vec<&str> = emails.iter().map(String::as_str).collect();
    fuzzy::match_list(query, &refs)
        .into_iter()
        .filter_map(|hit| person_from(hit, &emails, affinity))
        .collect()
}

fn person_from(hit: FuzzyHit, emails: &[String], affinity: &Affinity) -> Option<PersonHit> {
    let email = emails.get(hit.index)?.clone();
    let stats = affinity.get(&email)?.clone();
    Some(PersonHit {
        email,
        threads: stats.threads,
        replied: stats.replied,
        score: hit.score,
        indices: hit.indices,
    })
}

fn action_hits(commands: &[Command], query: &str) -> Vec<ActionHit> {
    let labels: Vec<&str> = commands
        .iter()
        .map(|command| command.label.as_str())
        .collect();
    fuzzy::match_list(query, &labels)
        .into_iter()
        .filter_map(|hit| {
            let command = commands.get(hit.index)?.clone();
            Some(ActionHit {
                command,
                score: hit.score,
                indices: hit.indices,
            })
        })
        .collect()
}

fn fuzzy_query(ranked: &RankedMail) -> String {
    let mut parts = ranked.parsed.query_words();
    parts.extend(ranked.parsed.phrases.iter().cloned());
    parts.join(" ")
}

#[cfg(test)]
#[path = "group_tests.rs"]
mod tests;
