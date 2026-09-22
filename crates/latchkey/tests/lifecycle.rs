//! Taking the lock, opening the door, and what is left when the agent goes.
//!
//! These run on the machine's real filesystem, in a temporary directory, because the properties
//! being checked are the ones the kernel provides — and a fake filesystem would be checking this
//! crate's idea of locking rather than the one it is built on.

#[cfg(unix)]
use latchkey::Endpoint;
use latchkey::{Agent, Environment, Error};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

/// An agent addressed inside a temporary directory, so tests never touch a real one.
///
/// `latchkey::here()` rather than a fixed `Host`: this file is about what the kernel does, so it
/// has to run under the rules of the machine it is on. The *rules themselves* are tested for all
/// three platforms in `address.rs`, where they are a pure function and need no kernel at all.
fn agent_in(dir: &std::path::Path) -> Agent {
    let dir = dir.as_os_str();
    Agent::in_environment(
        &unique_name(),
        latchkey::here(),
        &Environment {
            runtime_dir: Some(dir),
            tmpdir: Some(dir),
            local_app_data: Some(dir),
            user: Some("test"),
            ..Environment::default()
        },
    )
    .unwrap()
}

/// A name no other test, and no other run, will use.
///
/// A temporary directory is *not* enough isolation, and finding that out is what this file cost.
/// On Unix the endpoint lives inside the directory, so a per-test directory separates everything.
/// On Windows the endpoint is a named pipe, whose namespace is machine-wide and derives only
/// from the agent's name and the user — so every test here addressed `\\.\pipe\test-test`
/// while holding a different lock file, and thirteen agents fought over one door. The Unix suite
/// passed throughout; the Windows one hung.
///
/// The pid keeps concurrent `cargo test` runs apart, and the counter keeps this run's tests
/// apart from each other.
fn unique_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "t{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The socket, where there is one. `None` on Windows, where the endpoint is a pipe name.
#[cfg(unix)]
fn socket_of(agent: &Agent) -> Option<std::path::PathBuf> {
    match &agent.address().endpoint {
        Endpoint::Socket(path) => Some(path.clone()),
        Endpoint::Pipe(_) => None,
    }
}

mod one_per_user {
    use super::*;

    #[test]
    fn a_second_agent_is_refused() {
        // Two agents behind one name would each believe they were the only one, which for
        // anything holding state — a mailbox, a build cache, a connection pool — is two writers
        // and no arbitration.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let _first = agent.listen().expect("the first one becomes the agent");
        assert!(matches!(agent.listen(), Err(Error::AlreadyRunning)));
    }

    #[test]
    #[cfg(unix)]
    fn it_is_the_lock_that_refuses_and_not_the_socket() {
        // The distinction is the whole of this crate. Asking the *socket* whether an agent is
        // there means connecting, failing, and clearing the file — three steps with two windows
        // in them. Asking the *lock* is one step the kernel arbitrates.
        //
        // Shown by taking the socket away from underneath a running agent: the file is gone, so
        // any check that reads the filesystem would say the coast is clear, and the second agent
        // must still be refused.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let _first = agent.listen().unwrap();
        let socket = socket_of(&agent).expect("unix");
        std::fs::remove_file(&socket).unwrap();
        assert!(!socket.exists(), "the socket is gone and the agent is not");

        assert!(matches!(agent.listen(), Err(Error::AlreadyRunning)));
    }

    #[test]
    fn the_lock_goes_with_the_agent() {
        // The other half: an agent that has stopped must not lock its successor out. This is the
        // ordinary exit; `SIGKILL` is the same mechanism, because the kernel drops the lock with
        // the process whether or not any destructor ran.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        drop(agent.listen().unwrap());
        agent.listen().expect("the lock went with it");
    }

    #[test]
    fn the_lock_file_is_left_behind_on_purpose() {
        // Unlinking it would reintroduce the race in a worse form: a second process can hold the
        // lock on the very inode being deleted while a third locks a fresh file at the same
        // path, and the kernel is right both times. An empty file is the cheaper answer.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        drop(agent.listen().unwrap());
        assert!(agent.address().lock.exists());
        agent
            .listen()
            .expect("a leftover lock file locks nobody out");
    }

    #[test]
    fn asking_whether_one_is_running_does_not_start_one() {
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        assert!(!agent.is_running());
        let _live = agent.listen().unwrap();
        assert!(agent.is_running());
    }
}

mod the_door {
    use super::*;

    /// An agent that echoes one line back and goes away.
    fn echoing(agent: &Agent) -> std::thread::JoinHandle<()> {
        let door = agent.listen().expect("the door opens");
        std::thread::spawn(move || {
            if let Ok(mut client) = door.accept() {
                let mut line = String::new();
                let _ = BufReader::new(&mut client).read_line(&mut line);
                let _ = client.write_all(line.as_bytes());
                let _ = client.flush();
            }
        })
    }

    #[test]
    fn a_client_reaches_the_agent_and_is_answered() {
        // The whole point, in one test: the caller writes `Read`/`Write` and never learns which
        // transport carried it.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let served = echoing(&agent);

        let mut knocked = agent.connect().unwrap().expect("somebody is home");
        knocked.write_all(b"hello\n").unwrap();
        knocked.flush().unwrap();
        let mut back = String::new();
        BufReader::new(&mut knocked).read_line(&mut back).unwrap();
        assert_eq!(back, "hello\n");
        served.join().unwrap();
    }

    #[test]
    fn nobody_home_is_not_an_error() {
        // The usual reply to "no agent" is to start one, so it has to be distinguishable from a
        // failure without reading an error message.
        let dir = tempfile::tempdir().unwrap();
        assert!(agent_in(dir.path()).connect().unwrap().is_none());
    }

    #[test]
    #[cfg(unix)]
    fn a_socket_left_by_a_killed_agent_is_not_mistaken_for_a_live_one() {
        // The awkward case, and the only one that needs care: the process was killed, the file
        // is still there, and it looks exactly like an agent until you try to talk to it.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let socket = socket_of(&agent).expect("unix");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        // A raw listener, because this crate's guard unlinks on the way out and a killed process
        // runs no destructors. This is the residue of `kill -9`, not of an exit.
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        assert!(socket.exists(), "the stale file is the whole point");

        assert!(
            agent.connect().unwrap().is_none(),
            "a file with nothing behind it was taken for an agent"
        );
        agent
            .listen()
            .expect("a stale socket is cleared, not obeyed");
    }

    #[test]
    #[cfg(unix)]
    fn leaving_by_the_front_door_takes_the_socket_with_it() {
        // A socket left behind is what the next client mistakes for an agent. Nothing in the
        // program has to remember this — it is the guard's `Drop`, so every ordinary exit path
        // is covered by construction rather than by care.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let door = agent.listen().unwrap();
        let socket = socket_of(&agent).expect("unix");
        assert!(socket.exists());
        drop(door);
        assert!(!socket.exists(), "the socket outlived the agent");
    }
}

mod finding_or_starting {
    use super::*;

    #[test]
    fn a_running_agent_is_reused_rather_than_replaced() {
        // The property every `foo status` depends on: asking twice must not start a second
        // agent doing the same work twice.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let _door = agent.listen().unwrap();

        agent
            .connect_or_start(
                || panic!("it started one when another was already listening"),
                Duration::from_secs(1),
            )
            .expect("the running agent answers");
    }

    #[test]
    fn with_none_running_it_starts_one_and_waits() {
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let starting = agent.clone();
        let mut held = None;

        agent
            .connect_or_start(
                || {
                    held = Some(starting.listen().unwrap());
                    Ok(())
                },
                Duration::from_secs(5),
            )
            .expect("it started one and then reached it");
        assert!(held.is_some());
    }

    #[test]
    fn an_agent_that_never_answers_times_out_rather_than_hanging() {
        // A client blocked for ever on an agent that failed to start is worse than one that
        // gives up: the second can be retried by a person who can also read why.
        let dir = tempfile::tempdir().unwrap();
        let refused = agent_in(dir.path())
            .connect_or_start(|| Ok(()), Duration::from_millis(300))
            .expect_err("nothing ever listened");
        assert!(matches!(refused, Error::NeverAnswered(_)), "{refused}");
    }

    #[test]
    #[cfg(unix)]
    fn a_client_never_clears_a_socket_it_did_not_bind() {
        // The race this crate exists to close: a client that connects, fails, and *tidies* can
        // delete a socket another client's agent bound in between — two people running the same
        // command at the same moment on a cold machine is all it takes. The window cannot be hit
        // on demand, so what is asserted is the rule that removes it: a client leaves the
        // filesystem exactly as it found it, and clearing is `listen`'s job, under the lock.
        let dir = tempfile::tempdir().unwrap();
        let agent = agent_in(dir.path());
        let socket = socket_of(&agent).expect("unix");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());

        agent
            .connect_or_start(|| Ok(()), Duration::from_millis(200))
            .expect_err("nothing ever listened");
        assert!(
            socket.exists(),
            "the client deleted a socket that was not its own to delete"
        );
    }
}
