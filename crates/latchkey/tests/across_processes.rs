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
        // `Ok(None)` specifically. `Err(Busy)` means the agent is alive and occupied, so
        // reading "not reachable" as "stopped" would be exactly backwards.
        if matches!(agent.connect(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn demo(world: &World, command: &str) -> Command {
    let mut it = Command::new(the_example());
    it.arg(command)
        .env("LATCHKEY_DEMO_DIR", world.path())
        .env("LATCHKEY_DEMO_NAME", &world.name)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    it
}

/// A name no other test, and no other run, will use.
///
/// A temporary directory is not enough isolation. On Unix the endpoint lives inside it, so a
/// per-test directory separates everything; on Windows the endpoint is a named pipe whose
/// namespace is machine-wide and derives only from the name and the user. All three tests here
/// addressed `\\.\pipe\demo-demo` while holding three different locks, so they served each
/// other's clients — one failed and one hung for an hour and forty minutes.
fn unique_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "d{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Run a client and give back what it printed, without waiting on a pipe.
///
/// `Command::output()` would be the obvious thing and is the wrong thing here. It waits for the
/// child's stdout to reach end-of-file, and on Windows the agent the client starts inherits that
/// pipe and holds it open for as long as it runs — so the read finishes only when the *agent*
/// stops, which is never. See `latchkey::find::spawn` for why the standard library gives no
/// stable way to prevent the inheritance.
///
/// A file has no end-of-file to wait for, so redirecting there decouples the two. That is also
/// the first workaround `spawn`'s documentation recommends, which makes this test a check that
/// the advice works rather than only a way past the problem.
fn say(world: &World, command: &str, tag: &str) -> (std::process::ExitStatus, String, String) {
    let out = world.path().join(format!("{tag}.out"));
    let err = world.path().join(format!("{tag}.err"));
    let status = demo(world, command)
        .stdout(Stdio::from(std::fs::File::create(&out).unwrap()))
        .stderr(Stdio::from(std::fs::File::create(&err).unwrap()))
        .status()
        .expect("it runs");
    (
        status,
        std::fs::read_to_string(&out).unwrap_or_default(),
        std::fs::read_to_string(&err).unwrap_or_default(),
    )
}

/// One test's world: a private directory and a name nobody else uses.
struct World {
    dir: tempfile::TempDir,
    name: String,
}

impl World {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            name: unique_name(),
        }
    }

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }
}

/// The same agent the example addresses, so the test can knock on the door itself.
fn agent_at(world: &World) -> latchkey::Agent {
    let dir = world.path().as_os_str();
    latchkey::Agent::in_environment(
        &world.name,
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
    let world = World::new();
    let agent = agent_at(&world);
    let mut first = demo(&world, "serve").spawn().expect("it starts");
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
        let home = world.path().join(&world.name);
        assert!(
            home.join("agent.sock").exists(),
            "the residue is the whole point"
        );
        assert!(home.join("agent.lock").exists());
    }
    assert!(
        !matches!(agent.connect(), Ok(Some(_))),
        "the socket is still there and nothing is behind it"
    );

    let mut second = demo(&world, "serve").spawn().expect("it starts");
    let reached = answering(&agent, Duration::from_secs(5));
    let _ = second.kill();
    let _ = second.wait();
    assert!(reached, "the successor was locked out by a dead process");
}

#[test]
fn a_second_agent_in_a_second_process_is_refused() {
    let world = World::new();
    let agent = agent_at(&world);
    let mut first = demo(&world, "serve").spawn().expect("it starts");
    assert!(answering(&agent, Duration::from_secs(5)));

    // `output()` is safe here where it was not in `a_client_starts_an_agent_and_talks_to_it`:
    // this child is refused and exits immediately, and it starts nothing that could inherit the
    // pipe and outlive it.
    let second = demo(&world, "serve").output().expect("it runs");

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
    let world = World::new();
    let (first, said, complained) = say(&world, "ask", "first");
    // Asking again must reach the *same* agent, not start a second one. This is the property
    // every `foo status` depends on and the one a naive implementation loses first.
    let (second, twice, twice_complained) = say(&world, "ask", "second");

    // Tidy up *before* asserting, through the door rather than with a signal. An assertion that
    // fires first leaves a detached agent alive, and one still holding
    // `target/debug/examples/agent` open is one cargo cannot relink over — which is how the
    // first version of this test poisoned the next build rather than only failing it.
    stop(&agent_at(&world));

    assert!(first.success(), "{complained}");
    assert!(second.success(), "{twice_complained}");
    let pid: u32 = said
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("expected the agent's pid, got {said:?} / {complained}"));
    let same: u32 = twice
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("expected the agent's pid, got {twice:?} / {twice_complained}"));
    assert_eq!(pid, same, "the second client started its own agent");
}
