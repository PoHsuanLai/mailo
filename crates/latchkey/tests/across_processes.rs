//! The claims that are about processes, checked with processes.
//!
//! Everything in `lifecycle.rs` runs inside one program, which is enough for most of it because
//! an advisory lock conflicts between two file descriptions in the same process just as it does
//! between two processes. Two claims are not like that, and they are the two the crate is sold
//! on: that a *killed* agent locks nobody out, and that a client can start one it did not build.
//! Neither can be shown without a second process, so these tests drive `examples/agent.rs`.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where `cargo` puts the example this drives.
///
/// Derived from the test binary's own path — `target/<profile>/deps/across_processes-<hash>` —
/// because there is no `CARGO_BIN_EXE_` for examples. `cargo test` builds examples, so by the
/// time this runs it is there; if it is not, saying so beats a confusing spawn failure.
fn the_example() -> std::path::PathBuf {
    let here = std::env::current_exe().expect("the test binary knows where it is");
    let built = here
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<profile>/deps/<test>")
        .join("examples")
        .join(if cfg!(windows) { "agent.exe" } else { "agent" });
    assert!(
        built.exists(),
        "{} is missing; run `cargo test` rather than the test binary directly",
        built.display()
    );
    built
}

/// Ask an agent to stop, and wait until it has.
///
/// Every test that starts one ends here. A stray agent holds its example binary open, and on
/// Linux that is `ETXTBSY` for the next `cargo build` — a test that leaks one breaks the build
/// rather than just itself.
fn stop(agent: &latchkey::Agent) {
    use std::io::Write as _;
    if let Ok(Some(mut door)) = agent.connect() {
        let _ = writeln!(door, "stop");
        let _ = door.flush();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !matches!(agent.connect(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn demo(dir: &std::path::Path, command: &str) -> Command {
    let mut it = Command::new(the_example());
    it.arg(command)
        .env("LATCHKEY_DEMO_DIR", dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    it
}

/// The same agent the example addresses, so the test can knock on the door itself.
fn agent_at(dir: &std::path::Path) -> latchkey::Agent {
    let dir = dir.as_os_str();
    latchkey::Agent::in_environment(
        "demo",
        latchkey::here(),
        &latchkey::Environment {
            runtime_dir: Some(dir),
            tmpdir: Some(dir),
            local_app_data: Some(dir),
            user: Some("demo"),
            ..latchkey::Environment::default()
        },
    )
    .unwrap()
}

/// Wait until something actually answers.
///
/// Not "until the socket file exists", which is the check this whole crate argues against: after
/// a `kill -9` the file is there and nobody is home, so a test that waited on it would pass
/// against the bug it is meant to catch.
fn answering(agent: &latchkey::Agent, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if matches!(agent.connect(), Ok(Some(_))) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_killed_agent_locks_nobody_out() {
    // The claim the whole design rests on. A socket-based check would have to guess how long to
    // wait before deciding a leftover file is dead; the kernel releases an advisory lock with
    // the process, under `SIGKILL`, where no destructor runs and no timeout is needed.
    let dir = tempfile::tempdir().unwrap();
    let agent = agent_at(dir.path());
    let mut first = demo(dir.path(), "serve").spawn().expect("it starts");
    assert!(
        answering(&agent, Duration::from_secs(5)),
        "the first agent never opened its door"
    );

    // `Child::kill` is `SIGKILL` on Unix and `TerminateProcess` on Windows. Both are the case
    // that matters: no Rust code of ours runs on the way out, so nothing this crate wrote can be
    // what releases the lock. Both files are left behind, exactly as after a power cut.
    first.kill().unwrap();
    first.wait().unwrap();
    // The residue is Unix-specific: a named pipe is not a file and vanishes with the process
    // that made it, so on Windows there is nothing left to mistake for an agent. The claim being
    // tested — that the *lock* is released — is the same on both.
    #[cfg(unix)]
    {
        let socket = dir.path().join("demo").join("agent.sock");
        assert!(socket.exists(), "the residue is the whole point");
        assert!(dir.path().join("demo").join("agent.lock").exists());
    }
    assert!(
        !matches!(agent.connect(), Ok(Some(_))),
        "the socket is still there and nothing is behind it"
    );

    let mut second = demo(dir.path(), "serve").spawn().expect("it starts");
    let reached = answering(&agent, Duration::from_secs(5));
    let _ = second.kill();
    let _ = second.wait();
    assert!(reached, "the successor was locked out by a dead process");
}

#[test]
fn a_second_agent_in_a_second_process_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let agent = agent_at(dir.path());
    let mut first = demo(dir.path(), "serve").spawn().expect("it starts");
    assert!(answering(&agent, Duration::from_secs(5)));

    let second = demo(dir.path(), "serve").output().expect("it runs");

    assert!(!second.status.success(), "two agents answered to one name");
    let said = String::from_utf8_lossy(&second.stderr);
    assert!(said.contains("already running"), "{said}");
    assert!(
        answering(&agent, Duration::from_secs(1)),
        "the refused agent took the first one down with it"
    );
    let _ = first.kill();
    let _ = first.wait();
}

#[test]
fn a_client_starts_an_agent_and_talks_to_it() {
    // Demand-start end to end, across two processes that were not compiled together in any
    // sense that matters here: the client finds `current_exe`, launches it detached, waits for
    // the door, and gets an answer.
    let dir = tempfile::tempdir().unwrap();
    let asked = demo(dir.path(), "ask").output().expect("it runs");
    let said = String::from_utf8_lossy(&asked.stdout);
    let complained = String::from_utf8_lossy(&asked.stderr);
    assert!(asked.status.success(), "{complained}");

    let pid: u32 = said
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("expected the agent's pid, got {said:?} / {complained}"));

    // Asking again must reach the *same* agent, not start a second one. This is the property
    // every `foo status` depends on and the one a naive implementation loses first.
    let again = demo(dir.path(), "ask").output().expect("it runs");
    let same: u32 = String::from_utf8_lossy(&again.stdout)
        .trim()
        .parse()
        .unwrap();
    assert_eq!(pid, same, "the second client started its own agent");

    // Tidy up through the door rather than with a signal. A detached agent outlives the test
    // either way, but one still holding `target/debug/examples/agent` open is one cargo cannot
    // relink over — which is how the first version of this test poisoned the next build.
    stop(&agent_at(dir.path()));
}
