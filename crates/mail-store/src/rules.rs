//! Rules acting on mail: as it arrives, and on demand over what is already here.
//!
//! Written once, over the [`Store`] trait, so both stores run the same code and the parity tests
//! compare what they did rather than two implementations of it.
//!
//! Every action is the [`Op`] a person would apply by hand, through [`Op::apply`] and
//! [`Store::enqueue`]: it changes the message here, is undoable, and tells the server in the
//! same words — archiving on Gmail drops the Inbox label, on a server with folders it moves.
//! A rule is a shortcut for the user's own hand, never a second path to the same state.

use crate::{Store, StoreError};
use chrono::{DateTime, Utc};
use mail_domain::rule::{self, one_message};
use mail_domain::{
    AccountCaps, AccountId, AfterMatch, Body, Change, ChangeId, Filter, Label, LabelId,
    LabelOrigin, MailboxRole, MatchCtx, Membership, Message, MessageId, Op, PageReq, Patch,
    Property, Query, ReadState, Rule, RuleAction, RuleState, Sort, SortDir, Star, Target,
};

/// What running rules did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ran {
    /// Messages a rule acted on, each with the names of the rules that did, in order.
    pub acted: Vec<(MessageId, Vec<String>)>,
    /// Operations queued for the server.
    pub queued: usize,
    /// Messages looked at.
    pub examined: usize,
}

impl Ran {
    fn absorb(&mut self, other: Ran) {
        self.acted.extend(other.acted);
        self.queued += other.queued;
        self.examined += other.examined;
    }
}

/// Run the account's rules over mail that has just arrived.
///
/// `arrived` is what a sync pass stored for the first time — `SyncReport::arrived`, the same
/// list notifications are raised from — so a message is acted on once, when it first appears,
/// and never again on a later pass or on a message that was already here.
///
/// Only mail that arrived in the inbox: that is delivery, which is what a rule is about and all
/// a server's Sieve script ever sees. The user's own sent mail arriving in Sent is not mail a
/// rule should file away. A message gone again by the time rules run is skipped.
pub fn at_arrival<S: Store + ?Sized>(
    store: &S,
    account: AccountId,
    caps: &AccountCaps,
    arrived: &[MessageId],
    now: DateTime<Utc>,
) -> Result<Ran, StoreError> {
    let rules = store.rules(account)?;
    let ordered = rule::ordered(&rules);
    let mut ran = Ran::default();
    if ordered.is_empty() {
        return Ok(ran);
    }
    for id in arrived {
        let message = match store.message(*id) {
            Ok(message) => message,
            Err(StoreError::NoMessage(_)) => continue,
            Err(e) => return Err(e),
        };
        if message.account != account || message.mailbox != MailboxRole::Inbox {
            continue;
        }
        ran.absorb(act(store, caps, &ordered, message, now)?);
    }
    Ok(ran)
}

/// Run one rule over the mail already on an account, `batch` conversations at a time, calling
/// `progress` after each batch with what it did.
///
/// The rule runs whether or not it is enabled: being asked to is the point. Mail in Sent,
/// Drafts, Trash and Spam is left alone — the user's own mail, and mail already thrown away, is
/// not what "file what matches" means, and running a rule over Trash would bring it back out.
pub fn run_now<S: Store + ?Sized>(
    store: &S,
    caps: &AccountCaps,
    chosen: &Rule,
    batch: u32,
    now: DateTime<Utc>,
    progress: &mut dyn FnMut(&Ran),
) -> Result<Ran, StoreError> {
    let enabled = Rule {
        state: RuleState::Enabled,
        after: AfterMatch::Continue,
        ..chosen.clone()
    };
    let only = [&enabled];
    let mut total = Ran::default();
    let mut after = None;
    loop {
        let page = store.threads(
            &Query {
                filter: Filter::Account(chosen.account),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: after.clone(),
                    limit: batch.max(1),
                },
            },
            now,
        )?;
        let mut ran = Ran::default();
        for summary in &page.items {
            let thread = store.thread(summary.id)?;
            for id in &thread.messages {
                let message = store.message(*id)?;
                if matches!(
                    message.mailbox,
                    MailboxRole::Sent
                        | MailboxRole::Drafts
                        | MailboxRole::Trash
                        | MailboxRole::Spam
                ) {
                    continue;
                }
                ran.absorb(act(store, caps, &only, message, now)?);
            }
        }
        progress(&ran);
        total.absorb(ran);
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(total),
        }
    }
}

/// Run `rules`, already in order, over one message.
///
/// Each rule is tested against the message as the rules before it left it, as a Sieve script
/// would: a rule that marks mail read is seen by a later rule asking `is:read`.
fn act<S: Store + ?Sized>(
    store: &S,
    caps: &AccountCaps,
    rules: &[&Rule],
    mut message: Message,
    now: DateTime<Utc>,
) -> Result<Ran, StoreError> {
    let mut ran = Ran {
        examined: 1,
        ..Ran::default()
    };
    let mut fired = Vec::new();
    for rule in rules {
        let summary = one_message(&message);
        let corpus = match &message.body {
            Body::Present { text, .. } => text.as_deref(),
            Body::Absent => None,
        };
        // Its own addresses, re-read after each rule: a rule that files it elsewhere takes it
        // out of `InFolder` for the rules after, as it takes it out of a folder view.
        let folders = store.placed(message.id)?;
        let ctx = MatchCtx {
            summary: &summary,
            corpus,
            folders: &folders,
            now,
        };
        if !rule.fits(&ctx) {
            continue;
        }
        fired.push(rule.name.clone());
        for action in &rule.actions {
            let op = op_for(store, message.account, action)?;
            ran.queued += perform(store, caps, &op, &message, now)?;
            message = store.message(message.id)?;
        }
        if rule.after == AfterMatch::Stop {
            break;
        }
    }
    if !fired.is_empty() {
        ran.acted.push((message.id, fired));
    }
    Ok(ran)
}

/// The operation an action means, with any label it names found or made.
fn op_for<S: Store + ?Sized>(
    store: &S,
    account: AccountId,
    action: &RuleAction,
) -> Result<Op, StoreError> {
    Ok(match action {
        RuleAction::Label(name) => Op::Label(
            label_named(store, account, name, LabelOrigin::User)?,
            Membership::In,
        ),
        // The server's folder, so the server's origin: the user renames it there, not here.
        RuleAction::File(path) => {
            Op::File(label_named(store, account, path, LabelOrigin::Provider)?)
        }
        RuleAction::Archive => Op::Archive,
        RuleAction::Trash => Op::Trash,
        RuleAction::Spam => Op::Spam,
        RuleAction::MarkRead => Op::SetRead(ReadState::Read),
        RuleAction::Star => Op::SetStar(Star::Starred),
    })
}

/// The label called `name` on the account, created with `origin` if there is none.
///
/// The exact name first, then the same name in another case — a rule written `bills` should
/// find the `Bills` label rather than make a second one beside it.
fn label_named<S: Store + ?Sized>(
    store: &S,
    account: AccountId,
    name: &str,
    origin: LabelOrigin,
) -> Result<LabelId, StoreError> {
    let labels = store.labels(account)?;
    let found = labels
        .iter()
        .find(|l| l.name == name)
        .or_else(|| labels.iter().find(|l| l.name.eq_ignore_ascii_case(name)));
    if let Some(label) = found {
        return Ok(label.id);
    }
    let label = Label {
        id: LabelId::generate(),
        account,
        name: name.to_owned(),
        color: None,
        origin,
    };
    store.apply(
        account,
        &Patch {
            id: ChangeId::generate(),
            changes: vec![Change::LabelUpsert(label.clone())],
        },
    )?;
    Ok(label.id)
}

/// Apply `op` to one message and queue what the server must hear. Returns how many operations
/// were queued: none when the op changed nothing, or the server has nothing to be told.
fn perform<S: Store + ?Sized>(
    store: &S,
    caps: &AccountCaps,
    op: &Op,
    message: &Message,
    now: DateTime<Utc>,
) -> Result<usize, StoreError> {
    let thread = store.thread(message.thread)?;
    let messages = thread
        .messages
        .iter()
        .map(|id| store.message(*id))
        .collect::<Result<Vec<_>, _>>()?;
    let applied = op.apply(
        &Target::Messages(vec![message.id]),
        &thread,
        &messages,
        caps,
        now,
    );
    if applied.forward.changes.is_empty() {
        return Ok(0);
    }
    store.apply(message.account, &applied.forward)?;
    match applied.remote {
        Some(intent) => Ok(store
            .enqueue(message.account, intent, &applied.inverse, now)?
            .map_or(0, |_| 1)),
        None => Ok(0),
    }
}
