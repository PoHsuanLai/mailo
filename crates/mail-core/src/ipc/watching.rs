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

/// The name the watch's lock lives under, beside the daemon's.
const NAME: &str = "mailo-watch";

/// Proof that this process is the one watching. Dropping it, or the process ending, lets the next
/// one in.
#[derive(Debug)]
pub struct Watching {
    _held: latchkey::lock::Held,
}

/// Why this process may not watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Another `mailo watch` holds the lock.
    AlreadyWatching,
    /// The lock could not be taken for another reason.
    Failed(String),
}

fn agent() -> Result<latchkey::Agent, String> {
    latchkey::Agent::new(NAME).map_err(|e| e.to_string())
}

/// Become this user's watch, or say why not.
pub fn claim() -> Result<Watching, Refused> {
    let agent = agent().map_err(Refused::Failed)?;
    latchkey::lock::take(&agent.address().lock)
        .map(|held| Watching { _held: held })
        .map_err(|e| match e {
            latchkey::Error::AlreadyRunning => Refused::AlreadyWatching,
            other => Refused::Failed(other.to_string()),
        })
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
