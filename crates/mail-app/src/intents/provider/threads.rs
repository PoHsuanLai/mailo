//! Conversation actions: archive, star, label, snooze, and taking them back.
//!
//! The same [`mail_core::act`] the window performs with, so a conversation archived by an agent
//! is archived the way a click archives it: the patch, the work queued for the server, the undo.

use super::act::Thread;
use super::token::Token;
use super::{Provider, outcome};
use crate::intents::wire::{AppRefusal, EntityId, Invocation, Outcome, Target, UndoFault};
use chrono::Local;
use mail_core::undo::Undo;
use mail_core::{SqliteStore, Store};
use mail_domain::{LabelId, Membership, Op, Snooze, Star, ThreadId};

/// The conversations an action names, each as the thread it is, or the first that is not one.
pub(super) fn threads_of(invocation: &Invocation) -> Result<Vec<(EntityId, ThreadId)>, AppRefusal> {
    let Target::Entities(named) = &invocation.target else {
        return Err(AppRefusal::Unsupported);
    };
    named
        .iter()
        .map(|id| match (id.kind.as_str(), id.key.parse()) {
            ("mail.thread", Ok(key)) => Ok((id.clone(), ThreadId::from_uuid(key))),
            _ => Err(AppRefusal::NotFound(id.clone())),
        })
        .collect()
}

/// The label called `name`: exactly one, or why not.
fn label_named(store: &SqliteStore, name: &str) -> Result<LabelId, AppRefusal> {
    let found = mail_core::query::named(&mail_core::query::known_labels(store))(name);
    match found.as_slice() {
        [one] => Ok(*one),
        [] => Err(AppRefusal::Failed(format!(
            "there is no label called {name}"
        ))),
        _ => Err(AppRefusal::Failed(format!(
            "more than one label is called {name}"
        ))),
    }
}

/// The operation `kind` means, with its argument read from the invocation.
fn op_of(store: &SqliteStore, kind: Thread, invocation: &Invocation) -> Result<Op, AppRefusal> {
    let missing = |param: &str| AppRefusal::NeedsParam {
        param: param.to_owned(),
        options: Vec::new(),
    };
    match kind {
        Thread::Archive => Ok(Op::Archive),
        Thread::Star => Ok(Op::SetStar(Star::Starred)),
        Thread::Unstar => Ok(Op::SetStar(Star::Unstarred)),
        Thread::Label | Thread::Unlabel => {
            let name = invocation.text("label").ok_or_else(|| missing("label"))?;
            let membership = match kind {
                Thread::Unlabel => Membership::Out,
                _ => Membership::In,
            };
            Ok(Op::Label(label_named(store, name)?, membership))
        }
        Thread::Snooze => {
            let at = invocation
                .instant("until")
                .ok_or_else(|| missing("until"))?;
            chrono::DateTime::from_timestamp(at, 0)
                .map(|at| Op::SetSnooze(Snooze::Until(at)))
                .ok_or_else(|| AppRefusal::Failed("that is not a time".to_owned()))
        }
    }
}

impl Provider {
    /// Do `kind` to every conversation the invocation names, or to none of them: a part that
    /// cannot be done puts back the ones before it. What was done is one entry on the undo stack.
    pub(super) fn on_threads(
        &self,
        kind: Thread,
        invocation: &Invocation,
    ) -> Result<Outcome, AppRefusal> {
        let named = threads_of(invocation)?;
        let op = op_of(&self.store, kind, invocation)?;
        // Asked of every one first, so that a name that is gone refuses before anything changes.
        if let Some((id, _)) = named
            .iter()
            .find(|(_, thread)| self.store.thread(*thread).is_err())
        {
            return Err(AppRefusal::NotFound(id.clone()));
        }
        let mut done: Vec<Undo> = Vec::new();
        for (id, thread) in &named {
            match mail_core::act::perform(&self.store, *thread, op.clone()) {
                Some(undo) => done.push(undo),
                None => {
                    for part in done.iter().rev() {
                        let _ = mail_core::act::take_back(&self.store, part);
                    }
                    return Err(AppRefusal::Stale(id.clone()));
                }
            }
        }
        let said = mail_core::undo::said_of(&op, done.len(), &Local);
        let handle = self
            .stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_all(done);
        Ok(outcome(Some(said), handle.map(Token::Stack), None))
    }

    /// Take back one entry of the stack: every part, newest first. A part that is refused goes
    /// back on the stack as an entry of its own.
    pub(super) fn take_back_stack(
        &self,
        handle: mail_core::undo::UndoHandle,
    ) -> Result<(), UndoFault> {
        let mut stack = self
            .stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let parts = stack.take(handle).ok_or(UndoFault::Gone)?;
        let (_, refused): (Vec<Undo>, Vec<Undo>) = parts
            .into_iter()
            .rev()
            .partition(|part| mail_core::act::take_back(&self.store, part));
        if refused.is_empty() {
            return Ok(());
        }
        stack.push_all(refused.into_iter().rev().collect());
        Err(UndoFault::Conflict)
    }
}
