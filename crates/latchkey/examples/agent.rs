//! A minimal agent, and the client that finds it. Run two of these and watch one win.
//!
//! ```text
//! cargo run --example agent -- serve   # becomes the agent, or says one already is
//! cargo run --example agent -- ask     # reaches it, starting one if nobody answers
//! ```
//!
//! It is also what `tests/across_processes.rs` drives, because the crate's central claim — that
//! a killed agent locks nobody out — is about *processes*, and cannot be shown inside one.

use std::io::{BufRead, BufReader, Write};

fn main() {
    // `Display`, not the `Debug` that returning `Err` from `main` would print: "an agent is
    // already running for this user" is the sentence a person needs, and `AlreadyRunning` is not.
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // An address under `LATCHKEY_DEMO_DIR` when it is set, so a test can drive this without
    // touching the real per-user directory. A real program would just call `Agent::new`.
    let agent = match std::env::var_os("LATCHKEY_DEMO_DIR") {
        Some(dir) => latchkey::Agent::in_environment(
            "demo",
            latchkey::here(),
            &latchkey::Environment {
                runtime_dir: Some(&dir),
                tmpdir: Some(&dir),
                local_app_data: Some(&dir),
                user: Some("demo"),
                ..latchkey::Environment::default()
            },
        )?,
        None => latchkey::Agent::new("demo")?,
    };

    match std::env::args().nth(1).as_deref() {
        Some("serve") => {
            let door = agent.listen()?;
            println!("agent {} listening", std::process::id());
            for client in door.incoming() {
                // A client that hangs up mid-sentence is normal, not fatal. The first version of
                // this loop used `?` on the read and the write, so a client that connected and
                // closed — a health check, a port scan, a `stop` racing another — took the agent
                // down with it. An agent is a thing other people's code talks to badly.
                let Ok(mut client) = client else { continue };
                let mut line = String::new();
                if BufReader::new(&mut client).read_line(&mut line).is_err() {
                    continue;
                }
                match line.trim() {
                    "stop" => {
                        let _ = writeln!(client, "stopping");
                        return Ok(());
                    }
                    "" => continue, // knocked and left
                    _ => {
                        let _ = writeln!(client, "{}", std::process::id());
                    }
                }
            }
            Ok(())
        }
        Some("ask") => {
            let mut door = agent.connect_or_start(
                // `serve`, and nothing else: `spawn` launches `current_exe`, which for this
                // example is the built binary itself rather than anything cargo has to resolve.
                || latchkey::spawn(&["serve"]),
                std::time::Duration::from_secs(5),
            )?;
            writeln!(door, "who")?;
            door.flush()?;
            let mut said = String::new();
            BufReader::new(&mut door).read_line(&mut said)?;
            print!("{said}");
            Ok(())
        }
        _ => {
            eprintln!("usage: agent serve | agent ask");
            std::process::exit(2);
        }
    }
}
