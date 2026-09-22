//! The daemon end: bind the socket, answer clients, tidy up on the way out.
//!
//! What runs behind the door is still `sync::run`. Holding IDLE connections here — which is the
//! point of having a daemon at all — is the next step, and it is `sync::drive` in `Mode::Watch`
//! with its output going to clients instead of to a terminal.

use mail_store::SqliteStore;

/// What to do when a client asks for a pass.
///
/// A parameter rather than a call to `sync::run`, because the doorman should not know what is
/// behind the door: this file is the transport, and a test of it should not have to build a mail
/// engine in order to knock.
pub type Pass = Arc<dyn Fn(Arc<SqliteStore>) + Send + Sync>;
use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

/// Serve clients until told to stop — `mailo daemon`.
///
/// One connection at a time, deliberately. Every request here either answers immediately or
/// hands work to a thread, so a slow client cannot hold the door; concurrency belongs in the
/// sync engine, which already has it, rather than in the doorman.
pub fn serve(store: Arc<SqliteStore>, pass: Pass) -> Result<String, String> {
    let crate::ipc::Endpoint::Socket(path) = crate::ipc::endpoint()? else {
        return Err("named pipes are not implemented yet; this build is Unix only".to_owned());
    };
    let listener = crate::ipc::bind(&path)?;
    println!("listening on {}", path.display());

    // The socket outlives the process unless something removes it, and a file left behind is
    // what the next client mistakes for a daemon. Removed on the way out of every ordinary exit;
    // a kill -9 still leaves one, which is why `connect_or_start` tries before it tidies.
    let _tidy = Tidy(path.clone());

    for connection in listener.incoming() {
        let mut stream = match connection {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("a client could not be accepted: {e}");
                continue;
            }
        };
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() || line.is_empty() {
            continue;
        }
        let request = match crate::ipc::wire::parse::<crate::ipc::wire::Request>(&line) {
            Ok(request) => request,
            Err(crate::ipc::wire::Mismatch::Version { theirs, ours }) => {
                // Answered rather than dropped: a client from a different build needs to be told
                // which way round the mismatch is, and silence would look like a dead daemon.
                let _ = answer(
                    &mut stream,
                    crate::ipc::wire::Response::WrongVersion {
                        daemon: ours,
                        client: theirs,
                    },
                );
                continue;
            }
            Err(other) => {
                let _ = answer(
                    &mut stream,
                    crate::ipc::wire::Response::Refused(other.to_string()),
                );
                continue;
            }
        };

        let reply = match request {
            crate::ipc::wire::Request::Ping => crate::ipc::wire::Response::Pong {
                pid: std::process::id(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            crate::ipc::wire::Request::SyncNow => {
                // Answered before the pass, not after. A first sync takes minutes — `mailo sync`
                // against the real accounts measured 84 seconds — and a request that waited for
                // it would be a request nothing could cancel.
                // A thread, and the answer goes back before it finishes: a first sync measured
                // 84 seconds against the real accounts, and a doorman that waited for it would
                // answer nobody else meanwhile.
                let store = store.clone();
                let pass = pass.clone();
                std::thread::spawn(move || pass(store));
                crate::ipc::wire::Response::Started
            }
            crate::ipc::wire::Request::Shutdown => {
                let _ = answer(&mut stream, crate::ipc::wire::Response::Stopping);
                return Ok("stopped\n".to_owned());
            }
        };
        let _ = answer(&mut stream, reply);
    }
    Ok(String::new())
}

fn answer(
    stream: &mut std::os::unix::net::UnixStream,
    response: crate::ipc::wire::Response,
) -> Result<(), String> {
    let text = crate::ipc::wire::line(response)?;
    stream
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}

/// Removes the socket when the daemon leaves by any ordinary route.
struct Tidy(std::path::PathBuf);

impl Drop for Tidy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
