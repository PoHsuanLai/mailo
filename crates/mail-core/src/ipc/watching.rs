//! One `mailo watch` per user, and a way for everything else to tell whether it is running.
//!
//! `mailo watch` is the session's sync daemon (`dist/mailo-watch.service`, started at login), and
//! it announces new mail and shows the unread count on the launcher. A second one — a person
//! typing `mailo watch` in a terminal while the unit runs — would announce every message twice,
//! so it is refused. The window asks the same question to decide whether it should show the
//! launcher count itself: with a watch running, the watch is the one voice, and it keeps the count
//! when the window closes (a dock forgets a count when its sender leaves the bus).
//!
//! The answer is [`latchkey`]'s advisory lock, a kernel fact that dies with the process, the same
//! as the daemon's own (`ipc::agent`), under its own name.
//!
//! # A door for the window
//!
//! The watch is the process that stores pushed mail while a window is open, so it is the one
//! that can say the moment it has. It listens behind its lock, as the daemon does behind its
//! own, and answers [`Request::Ping`] and [`Request::Subscribe`]; [`Watching::changed`] is how
//! its passes say what they ran. The rest is the daemon's to do and is refused here: a watch
//! fetches on its own schedule, and is stopped by its unit or by Ctrl-C.

use super::changes::{self, Answer, Subscribers};
use super::wire::{Request, Response};
use mail_domain::AccountId;

/// The name the watch's lock and door live under, beside the daemon's.
const NAME: &str = "mailo-watch";

/// Proof that this process is the one watching, and the listeners its passes tell.
///
/// The lock is held by the door's thread, which serves for as long as the process runs: the
/// process ending is what lets the next one in.
#[derive(Debug)]
pub struct Watching {
    changes: Subscribers,
}

impl Watching {
    /// A pass on `account` has ended and may have stored something. Never blocks.
    pub fn changed(&self, account: AccountId) {
        self.changes.tell(account);
    }

    /// How many are listening. For tests, which need to know a listener has arrived before
    /// telling it anything.
    pub fn listeners(&self) -> usize {
        self.changes.count()
    }
}

/// Why this process may not watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Another `mailo watch` holds the lock.
    AlreadyWatching,
    /// The lock could not be taken, or the door opened, for another reason.
    Failed(String),
}

/// This user's watch, wherever this platform puts such a thing: where its lock is, and the door
/// the window knocks on.
pub fn agent() -> Result<latchkey::Agent, String> {
    latchkey::Agent::new(NAME).map_err(|e| e.to_string())
}

/// Become this user's watch, or say why not.
pub fn claim() -> Result<Watching, Refused> {
    claim_at(&agent().map_err(Refused::Failed)?)
}

/// Become the watch at `agent`'s address: [`claim`], for a test that keeps its own.
pub fn claim_at(agent: &latchkey::Agent) -> Result<Watching, Refused> {
    let listening = agent.listen().map_err(|e| match e {
        latchkey::Error::AlreadyRunning => Refused::AlreadyWatching,
        other => Refused::Failed(other.to_string()),
    })?;
    let changes = Subscribers::default();
    let serving = changes.clone();
    std::thread::Builder::new()
        .name("mailo-watch-door".to_owned())
        .spawn(move || {
            changes::door(&listening, &serving, |request| match request {
                Request::Ping => Answer::Say(changes::pong()),
                Request::Subscribe => Answer::Subscribe,
                Request::SyncNow => Answer::Say(Response::Refused(
                    "a watch fetches on its own and runs no pass on request".to_owned(),
                )),
                Request::Shutdown => Answer::Say(Response::Refused(
                    "a watch is stopped where it was started, not through its door".to_owned(),
                )),
            });
        })
        .map_err(|e| Refused::Failed(e.to_string()))?;
    Ok(Watching { changes })
}

/// Whether a watch is running now, for this user.
pub fn running() -> bool {
    agent().is_ok_and(|agent| agent.is_running())
}

#[cfg(test)]
mod tests {
    // One test, because the lock is per user and tests share a process: a second test claiming it
    // at the same moment would see the first's. The environment is the scratch one's, never the
    // person's runtime directory.
    #[test]
    fn one_watch_per_user_and_the_lock_goes_with_it() {
        let scratch = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        // The address the lock would have in a runtime directory of its own, asked for rather than
        // reached by changing this process's environment.
        let environment = latchkey::Environment {
            runtime_dir: Some(scratch.path().as_os_str()),
            tmpdir: None,
            home: None,
            local_app_data: None,
            user: None,
        };
        let agent = latchkey::Agent::in_environment(
            "mailo-watch-test",
            latchkey::Host::Linux,
            &environment,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(!agent.is_running());
        let first = latchkey::lock::take(&agent.address().lock).unwrap_or_else(|e| panic!("{e}"));
        assert!(agent.is_running(), "held, so running");
        assert!(matches!(
            latchkey::lock::take(&agent.address().lock),
            Err(latchkey::Error::AlreadyRunning)
        ));
        drop(first);
        assert!(!agent.is_running(), "the lock went with its holder");
    }
}
