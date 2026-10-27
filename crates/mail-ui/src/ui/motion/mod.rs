//! Motion keyed to state.
//!
//! Every animation here starts because something the window already knows became true, never
//! because a timer stands in for it, and every one is quire's, timed by quire's motion clock
//! (`ds::prelude::settle`), never by an `animationend` event (coherence rule 4):
//!
//! - a row leaves because an op was applied and the row no longer belongs to the list: quire's
//!   `List` plays the exit for a key that stops being listed and closes the gap;
//! - the undo toast is up because an op was applied, and it names it (quire's `ToastHost`).
//!
//! **The row leaves on the press.** The write goes to the client (`mail_client::Command`), which
//! applies it off the thread that draws; the list stops drawing what it takes out at once
//! (`list_query::hide`), and the undo and the toast that names it come when it is done.

pub(super) mod drag;
mod toast;

pub(super) use drag::Ghost;
pub(super) use toast::Toast;

use super::ops::resolve;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::stack::toast_hub::{ToastAction, ToastHub, UndoToken};
use mail_client::Client;
use mail_client::undo::{Undo, UndoHandle};
use mail_client::{Acted, Command, Done, Ticket};
use mail_domain::*;

/// What the toast says, and which op it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Said {
    pub text: String,
    pub serial: u64,
    pub follow: Follow,
}

/// What the toast offers beside its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Follow {
    /// The tab that takes back the op the stack holds under this handle.
    Undo(UndoHandle),
    /// After leaving a list: archive what it already sent, by its sender's address.
    ArchiveFrom { sender: String, list: String },
    /// After a sign-in in the browser that did not work: try it again, or open the add-account
    /// window, which says what is missing.
    SignIn(crate::ui::add_account::Retry),
    /// Undo for something deleted in Settings that is not mail: put it back as it was.
    Restore(Restore),
    /// After Block Sender: take back the conversation's move to Spam (`undo`, when it moved) and
    /// delete the rule the block made (`rule`, none when the sender was blocked already, so that
    /// rule is not this gesture's to remove).
    Unblock {
        rule: Option<Rule>,
        undo: Option<UndoHandle>,
    },
    /// Nothing to take back: an unsubscribe, once made, is the list's.
    Nothing,
}

/// Something deleted in Settings that a toast's Undo puts back as it was. Mail goes on the undo
/// stack as a patch; these are records of their own, so each keeps the whole record instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Restore {
    Rule(Rule),
    Key(PgpKey),
    Cert(SmimeCert),
    Signature(Signature),
    /// A redirect still in its grace period: Undo withdraws it.
    Redirect(mail_client::compose::Redirected),
}

impl Restore {
    /// Put it back in `client`, or say why not.
    pub(in crate::ui) fn put_back(&self, client: &Client) -> Result<(), String> {
        match self {
            Restore::Rule(rule) => client.rules_manage().restore(rule).map_err(not_put_back),
            Restore::Key(key) => client.keys().restore(key).map_err(not_put_back),
            Restore::Cert(cert) => client.certs().restore(cert).map_err(not_put_back),
            Restore::Signature(had) => client.signing().restore(had).map_err(not_put_back),
            Restore::Redirect(redirected) => client
                .compose()
                .withdraw_redirect(redirected)
                .map_err(not_put_back),
        }
    }
}

/// What Undo says when the client would not take back what it deleted.
fn not_put_back(why: impl std::fmt::Display) -> String {
    crate::ui::remedy::plainly(crate::ui::copy::errors::NOT_KEPT_HERE, why)
}

/// quire's toast host, and what its buttons do: made by a component that lives as long as the
/// window, because the handler belongs to the scope that made it and must outlive the toast.
#[derive(Clone, Copy)]
pub(super) struct Toasts {
    pub hub: ToastHub,
    pub on_undo: EventHandler<UndoToken>,
    /// What the "Archive All" button of a leave-a-list toast does: it reads the list it offered
    /// from [`Motion::toast`] when pressed.
    pub on_archive: EventHandler<()>,
    /// What the button of a sign-in that did not work does ("Try Again", "Use Add Account…"):
    /// it reads what it offered from [`Motion::toast`] when pressed.
    pub on_sign_in: EventHandler<()>,
    /// What the Undo of a deletion in Settings does: it reads what to put back from
    /// [`Motion::toast`] when pressed.
    pub on_restore: EventHandler<()>,
    /// What the Undo of Block Sender does: it reads the rule and the move to take back from
    /// [`Motion::toast`] when pressed.
    pub on_unblock: EventHandler<()>,
}

/// The motion state the window shares.
#[derive(Clone, Copy)]
pub(super) struct Motion {
    /// The follow-up a toast on screen offers that is not an undo (archive what a list already
    /// sent, or take a block back), kept here for its button's handler to read. The toast
    /// itself is quire's, through [`Motion::toasts`].
    pub toast: Signal<Option<Said>>,
    /// The window root's toast host and the handlers its buttons call, once the list has mounted
    /// under the root. Not reactive: only a toast being said reads it.
    pub toasts: CopyValue<Option<Toasts>>,
    /// The place a hovered Archive or Snooze button would send the row to.
    pub dest: Signal<Option<&'static str>>,
    pub drag: Signal<drag::Drag>,
    /// The ids the list drew last, in order. Not reactive: only an op reads it.
    pub order: CopyValue<Vec<ThreadId>>,
    /// The message each of `order`'s rows is, while the list gives a message to a row; empty
    /// otherwise.
    pub messages: CopyValue<Vec<MessageId>>,
}

/// Make the motion state for the window. Called once, from `App`.
pub(super) fn use_motion() -> Motion {
    use_context_provider(|| Motion {
        toast: Signal::new(None),
        toasts: CopyValue::new(None),
        dest: Signal::new(None),
        drag: Signal::new(drag::Drag::Idle),
        order: CopyValue::new(Vec::new()),
        messages: CopyValue::new(Vec::new()),
    })
}

/// The window's motion state, when there is a window around the caller.
pub(super) fn motion() -> Option<Motion> {
    try_consume_context::<Motion>()
}

/// Apply what a button means, and let the window show it.
pub(super) fn act_kind(
    client: &Client,
    shell: Signal<Shell>,
    thread: ThreadId,
    kind: OpKind,
) -> bool {
    resolve(client, thread, kind).is_some_and(|op| act(client, shell, thread, op))
}

/// Apply `op` to `thread` now, keep its undo, and start whatever motion it means.
pub(super) fn act(client: &Client, shell: Signal<Shell>, thread: ThreadId, op: Op) -> bool {
    act_all(client, shell, vec![(thread, op)]) > 0
}

/// Apply each op to its conversation now, as one gesture: one undo entry that takes every one
/// of them back, and one toast that counts them. Returns how many were applied.
///
/// `Op::apply` is about one thread, so the ops are applied one by one; what makes them one
/// gesture is that their undos are kept together (`UndoStack::push_all`). A row that no longer
/// belongs to the list leaves with it: quire's `List` plays that exit.
pub(super) fn act_all(client: &Client, shell: Signal<Shell>, ops: Vec<(ThreadId, Op)>) -> usize {
    act_saying(client, shell, ops, None)
}

/// [`act_all`], its toast saying `instead` rather than what the ops did, when there is more to
/// say than the op: the toast goes up once the ops are done, so words put up before then would
/// be replaced by the op's own.
pub(in crate::ui) fn act_saying(
    _store: &Client,
    shell: Signal<Shell>,
    ops: Vec<(ThreadId, Op)>,
    instead: Option<String>,
) -> usize {
    if ops.is_empty() {
        return 0;
    }
    let count = ops.len();
    let asked: Vec<ThreadId> = ops.iter().map(|(thread, _)| *thread).collect();
    let hours = crate::ui::clock::hours();
    let hiding = super::list_query::hide(&ops);
    let alone: Vec<(ThreadId, mail_domain::MessageId)> = asked
        .iter()
        .filter_map(|thread| match target_of(shell, *thread) {
            Target::Messages(messages) => messages.first().map(|message| (*thread, *message)),
            Target::Threads(_) => None,
        })
        .collect();
    let then_erase = deletes_now();
    let ticket = super::live::client().act(Command::Perform {
        ops,
        clock: hours,
        alone,
        then_erase,
    });
    let motion = motion();
    if let Some(mut motion) = motion {
        motion.dest.set(None);
    }
    when_done(ticket, move |acted| {
        let Some(Acted {
            done: Done::Performed(done),
            generation,
        }) = acted
        else {
            hiding.settle(None, &[]);
            return 0;
        };
        // What was refused stays where it was, so it is drawn again.
        let refused: Vec<ThreadId> = asked
            .iter()
            .filter(|thread| !done.iter().any(|(applied, ..)| applied == *thread))
            .copied()
            .collect();
        hiding.settle(Some(generation), &refused);
        performed(motion, shell, done, hours, instead)
    })
    .unwrap_or(count)
}

/// Keep what `done` takes back as one entry and say it: what an op applied comes to in the
/// window. Returns how many were applied.
fn performed(
    motion: Option<Motion>,
    shell: Signal<Shell>,
    done: Vec<(ThreadId, Op, Undo)>,
    hours: mail_client::when::Clock,
    instead: Option<String>,
) -> usize {
    let count = done.len();
    let said = match done.as_slice() {
        [] => return 0,
        [(_, op, _), _, ..] => mail_client::undo::said_of(op, count, &chrono::Local, hours),
        [(_, _, undo)] => undo.said.clone(),
    };
    let said = instead.unwrap_or(said);
    let undos = done.into_iter().map(|(.., undo)| undo).collect();
    if record_through(motion, shell, undos, said).is_none() {
        return 0;
    }
    count
}

/// What an action on `thread` reaches: the one message the reader shows alone, when `thread` is
/// the open conversation opened from a message's own row and nothing is picked, else the
/// conversation whole. A picked row is a conversation, whichever of its messages' rows it is.
fn target_of(shell: Signal<Shell>, thread: ThreadId) -> Target {
    let current = shell.peek();
    match current.focus {
        Some(message)
            if current.open == Some(thread)
                && current
                    .picked
                    .chosen(&super::picks::drawn_order())
                    .is_empty() =>
        {
            Target::Messages(vec![message])
        }
        _ => Target::Threads(vec![thread]),
    }
}

/// Whether Delete means "deletes now" in this window's settings. A window with none (a test's)
/// moves to Trash and no more.
fn deletes_now() -> bool {
    try_consume_context::<Signal<crate::settings::MailSettings>>().is_some_and(|settings| {
        settings.peek().mailboxes.delete == crate::settings::DeleteMeans::Now
    })
}

/// Hand what `ticket` says to `then` once it is done: here and now when it already is (a client
/// that applies on this thread), otherwise from a task of the window's that outlives the
/// component that asked, so a row that has left still has its undo kept. Returns what `then`
/// did, when it ran here.
pub(in crate::ui) fn when_done<T: 'static>(
    mut ticket: Ticket,
    then: impl FnOnce(Option<Acted>) -> T + 'static,
) -> Option<T> {
    if let Some(acted) = ticket.now() {
        return Some(then(Some(acted)));
    }
    dioxus::dioxus_core::spawn_forever(async move {
        then(ticket.await);
    });
    None
}

/// The one way an undoable action is kept and shown: `undos` go on the window's stack as one
/// entry, taken back together by ⌘Z or by the toast's Undo, and `said` goes up in that toast.
/// Nothing to undo is still said, in plain words, and keeps no entry.
pub(in crate::ui) fn record(
    shell: Signal<Shell>,
    undos: Vec<Undo>,
    said: String,
) -> Option<UndoHandle> {
    record_through(motion(), shell, undos, said)
}

/// [`record`], through motion state looked up earlier: for what is kept once a write the
/// window gave the client is done, from a task that no longer finds the window's.
pub(in crate::ui) fn record_through(
    motion: Option<Motion>,
    mut shell: Signal<Shell>,
    undos: Vec<Undo>,
    said: String,
) -> Option<UndoHandle> {
    let handle = shell.write().undo.push_all(undos);
    if let Some(motion) = motion {
        motion.say(said, handle.map_or(Follow::Nothing, Follow::Undo));
    }
    handle
}

/// Apply what a button means to each of `threads`, as one gesture ([`act_all`]).
///
/// Resolved per conversation, and only where that conversation allows it (`mail_client::conversation::offers`):
/// archiving a selection that holds something already archived archives the rest and leaves
/// that one be, and its undo does not "restore" what the gesture never moved.
pub(super) fn act_kind_all(
    client: &Client,
    shell: Signal<Shell>,
    threads: &[ThreadId],
    kind: OpKind,
) -> usize {
    let ops = threads
        .iter()
        .filter_map(|thread| {
            let loaded = client.stored().thread(*thread).ok()?;
            mail_client::conversation::offers(&loaded.summary, kind)
                .then(|| resolve(client, *thread, kind))
                .flatten()
                .map(|op| (*thread, op))
        })
        .collect();
    act_all(client, shell, ops)
}

/// Delete forever what each of `threads` holds in `bin`, as one gesture the user has already
/// confirmed. Nothing goes on the undo stack, and what is there about those messages is
/// forgotten, so ⌘Z cannot claim to bring any of them back. Returns how many messages were
/// deleted. The rows leave with the list: quire's `List` plays the exit, and the toast has no
/// Undo.
pub(in crate::ui) fn destroy_all(
    _store: &Client,
    mut shell: Signal<Shell>,
    threads: &[ThreadId],
) -> usize {
    if threads.is_empty() {
        return 0;
    }
    let ops: Vec<(ThreadId, Op)> = threads
        .iter()
        .map(|thread| (*thread, Op::Destroy))
        .collect();
    let hiding = super::list_query::hide(&ops);
    let ticket = super::live::client().act(Command::Destroy(threads.to_vec()));
    let asked = threads.len();
    let threads = threads.to_vec();
    let motion = motion();
    when_done(ticket, move |acted| {
        let Some(Acted {
            done: Done::Destroyed(gone),
            generation,
        }) = acted
        else {
            hiding.settle(None, &[]);
            return 0;
        };
        hiding.settle(Some(generation), &[]);
        if gone.is_empty() {
            return 0;
        }
        {
            let mut shell = shell.write();
            shell.undo.forget_messages(&gone);
            if shell.open.is_some_and(|open| threads.contains(&open)) {
                shell.close();
            }
        }
        if let Some(motion) = motion {
            motion.say(crate::ui::bin::said(gone.len()), Follow::Nothing);
        }
        gone.len()
    })
    .unwrap_or(asked)
}

/// Put up the toast for something that is not an op on one row.
pub(in crate::ui) fn tell(text: String, follow: Follow) {
    if let Some(motion) = motion() {
        motion.say(text, follow);
    }
}

/// [`tell`], through motion state looked up earlier: for a task that outlives the component
/// that started it, where the lookup would no longer find the window's.
pub(in crate::ui) fn tell_through(motion: Option<Motion>, text: String) {
    if let Some(motion) = motion {
        motion.say(text, Follow::Nothing);
    }
}

/// Say `text` through motion state looked up earlier, with an Undo that puts `restore` back: for
/// a task that outlives the component that started it. Hands the words back when there is no
/// window to say them in.
pub(in crate::ui) fn offer_restore(
    motion: Option<Motion>,
    text: String,
    restore: Restore,
) -> Result<(), String> {
    match motion {
        Some(motion) => {
            motion.say(text, Follow::Restore(restore));
            Ok(())
        }
        None => Err(text),
    }
}

/// Block Sender's second half: file `thread` in Spam as Mark as Spam does, and say `text` once,
/// with the one Undo that takes back both that move and `rule`, the rule just made
/// ([`Follow::Unblock`]).
pub(in crate::ui) fn spam_and_offer_unblock(
    client: &Client,
    mut shell: Signal<Shell>,
    thread: ThreadId,
    rule: Option<Rule>,
    text: String,
) {
    let hours = crate::ui::clock::hours();
    let moved = resolve(client, thread, OpKind::Spam)
        .and_then(|op| client.acting().perform(thread, op, hours))
        .and_then(|undo| shell.write().undo.push_all(vec![undo]));
    tell(text, Follow::Unblock { rule, undo: moved });
}

/// Take back the newest op: ⌘Z.
pub(super) fn undo_last(client: &Client, mut shell: Signal<Shell>) -> bool {
    let Some(entry) = shell.write().undo.pop() else {
        return false;
    };
    restore(client, shell, motion(), entry)
}

/// Take back the op the toast named, by the handle its undo carried: the toast's tab.
pub(super) fn undo_by(
    client: &Client,
    mut shell: Signal<Shell>,
    motion: Option<Motion>,
    handle: UndoHandle,
) -> bool {
    let Some(entry) = shell.write().undo.take(handle) else {
        return false;
    };
    restore(client, shell, motion, entry)
}

/// Put `entry` back, every part of it, and take down the toast that offered it. A part that
/// is refused goes back on the stack, as an entry of its own gesture's parts.
fn restore(
    client: &Client,
    mut shell: Signal<Shell>,
    motion: Option<Motion>,
    entry: Vec<Undo>,
) -> bool {
    // Newest first, as the parts were done oldest first. A send taken back goes again for its
    // time: the window seals and queues it. The rest is the client's to take back.
    let (sends, parts): (Vec<Undo>, Vec<Undo>) = entry
        .into_iter()
        .rev()
        .partition(|part| part.resend.is_some());
    let (sent, unsent): (Vec<Undo>, Vec<Undo>) = sends.into_iter().partition(|part| {
        part.resend
            .is_some_and(|resend| crate::ui::compose::send_again(client, resend))
    });
    let asked = !sent.is_empty() || !parts.is_empty();
    let mut took = move |refused: Vec<Undo>| {
        let _ = shell
            .write()
            .undo
            .push_all(refused.into_iter().rev().collect());
    };
    if parts.is_empty() {
        took(unsent);
    } else {
        let ticket = super::live::client().act(Command::TakeBack(parts.clone()));
        when_done(ticket, move |acted| {
            let refused = match acted {
                Some(Acted {
                    done: Done::TookBack { refused, .. },
                    ..
                }) => refused,
                // Not done: all of it is still there to take back.
                _ => parts,
            };
            took(unsent.into_iter().chain(refused).collect());
        });
    }
    if !asked {
        return false;
    }
    // A row listed again while it leaves stays where it was (quire's `List`), and one whose
    // exit has settled enters again: the list is told nothing.
    if let Some(mut motion) = motion {
        motion.toast.set(None);
        if let Some(toasts) = *motion.toasts.peek() {
            toasts.hub.hide();
        }
    }
    true
}

/// The keys motion owns: Esc drops a drag, undo undoes, and the hover card takes Space and
/// Esc. `undo` is whether the keymap resolved the press to the standard Undo action (⌘Z on a
/// Mac or our desktop, Ctrl+Z elsewhere). Returns whether the key was handled. Never while
/// typing: undo in a field is the field's own.
pub(super) fn key(name: &str, undo: bool, typing: bool, shell: Signal<Shell>) -> bool {
    if name == "Escape" && motion().is_some_and(drag::cancel) {
        return true;
    }
    if typing {
        return false;
    }
    if undo {
        let client = crate::ui::live::client();
        undo_last(&client, shell);
        return true;
    }
    false
}

impl Motion {
    /// Put up the toast. The next op replaces it; otherwise it leaves on its own. All of them are
    /// quire's: an undo, plain words, or one button of the follow-up's own ("Archive All", or
    /// "Undo" for a block).
    fn say(mut self, text: String, follow: Follow) {
        let toasts = *self.toasts.peek();
        match (follow, toasts) {
            (Follow::Undo(handle), Some(toasts)) => {
                self.toast.set(None);
                toasts
                    .hub
                    .push_undoable(text, UndoToken(handle.0), toasts.on_undo);
            }
            (Follow::Nothing, Some(toasts)) => {
                self.toast.set(None);
                toasts.hub.push(text, None);
            }
            (follow @ Follow::ArchiveFrom { .. }, Some(toasts)) => {
                let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
                self.toast.set(Some(Said {
                    text: text.clone(),
                    serial,
                    follow,
                }));
                toasts
                    .hub
                    .push_action(text, ToastAction::new("Archive All"), toasts.on_archive);
            }
            (follow @ Follow::Restore(_), Some(toasts)) => {
                let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
                self.toast.set(Some(Said {
                    text: text.clone(),
                    serial,
                    follow,
                }));
                toasts.hub.push_action(
                    text,
                    ToastAction::new(crate::ui::copy::controls::UNDO),
                    toasts.on_restore,
                );
            }
            (follow @ Follow::Unblock { .. }, Some(toasts)) => {
                let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
                self.toast.set(Some(Said {
                    text: text.clone(),
                    serial,
                    follow,
                }));
                toasts.hub.push_action(
                    text,
                    ToastAction::new(crate::ui::copy::controls::UNDO),
                    toasts.on_unblock,
                );
            }
            (Follow::SignIn(retry), Some(toasts)) => {
                let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
                let label = retry.label();
                self.toast.set(Some(Said {
                    text: text.clone(),
                    serial,
                    follow: Follow::SignIn(retry),
                }));
                toasts
                    .hub
                    .push_action(text, ToastAction::new(label), toasts.on_sign_in);
            }
            // No host yet (a window still mounting): the words are kept, as before.
            (follow, None) => {
                let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
                self.toast.set(Some(Said {
                    text,
                    serial,
                    follow,
                }));
            }
        }
    }
}

/// Whether `thread`, as the client now holds it, is still in the list `shell` shows.
///
/// A search is global, so a row there stays whatever happened to it; a place is its filter,
/// asked of the thread with the folders the server holds its messages in, so a folder's place
/// keeps a row that was starred and lets go of one whose mail has left the folder. What the
/// list draws is the client's own answer to that query; this asks it of one thread, for the
/// tests that pin the two to each other.
#[cfg(test)]
pub(in crate::ui) fn belongs(
    client: &Client,
    shell: &Shell,
    thread: ThreadId,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !shell.search.trim().is_empty() {
        return true;
    }
    let Ok(after) = client.stored().thread(thread) else {
        return false;
    };
    let folders: Vec<Placed> = after
        .messages
        .iter()
        .filter_map(|message| client.stored().placed(*message).ok())
        .flatten()
        .collect();
    shell.query(1).filter.fit(&MatchCtx {
        summary: &after.summary,
        corpus: None,
        folders: &folders,
        sizes: &[],
        now,
    })
}

#[cfg(test)]
mod tests;
