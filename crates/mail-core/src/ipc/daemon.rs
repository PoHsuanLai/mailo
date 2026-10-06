//! The daemon end: bind the socket, answer clients, tidy up on the way out.
//!
//! What runs behind the door is still `sync::run`. Holding IDLE connections here — which is the
//! point of having a daemon at all — is the next step, and it is `sync::drive` in `Mode::Watch`
//! with its output going to clients instead of to a terminal. Until then a `mailo watch` holds
//! them, and keeps a door of its own for the window to listen at (`super::watching`).

use super::changes::{self, Answer, Subscribers};
use super::wire::{Request, Response};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// What to do when a client asks for a pass, returning the accounts whose pass may have stored
/// something (`PassEnd::may_have_stored`), which are what the subscribers are told of.
///
/// A parameter rather than a call to `sync::run`, because the doorman should not know what is
/// behind the door: this file is the transport, and a test of it should not have to build a mail
/// engine in order to knock.
pub type Pass = Arc<dyn Fn(Arc<SqliteStore>) -> Vec<AccountId> + Send + Sync>;

/// Serve clients until told to stop — `mailo daemon`.
///
/// One connection at a time; see [`changes::door`] for why that is enough.
pub fn serve(store: Arc<SqliteStore>, pass: Pass) -> Result<String, String> {
    let agent = crate::ipc::agent()?;
    // The lock inside this value is what makes "one daemon per user" true, and dropping it is
    // what removes the socket, so it is held for the whole of `serve`. `Err(AlreadyRunning)` is
    // the answer to "should I start?", and it is a kernel fact rather than a guess about a file.
    let listening = agent.listen().map_err(|e| match e {
        // Said in this program's words. `latchkey` has to call it an agent because it does not
        // know what it is holding the door for; here it is a daemon, and the remedy is a command
        // the reader can type.
        latchkey::Error::AlreadyRunning => {
            "a mailo daemon is already running; `mailo daemon --stop` will stop it".to_owned()
        }
        other => other.to_string(),
    })?;
    match agent.socket() {
        Some(path) => println!("listening on {}", path.display()),
        None => println!("listening on {}", agent.address().endpoint),
    }

    let subscribers = Subscribers::default();
    let mut stopped = false;
    changes::door(&listening, &subscribers, |request| match request {
        Request::Ping => Answer::Say(changes::pong()),
        Request::SyncNow => {
            // A thread, and the answer goes back before it finishes: a first sync measured
            // 84 seconds against the real accounts, and a doorman that waited for it would
            // answer nobody else meanwhile — and a request that waited would be one nothing
            // could cancel.
            let store = store.clone();
            let pass = pass.clone();
            let subscribers = subscribers.clone();
            std::thread::spawn(move || {
                for account in pass(store) {
                    subscribers.tell(account);
                }
            });
            Answer::Say(Response::Started)
        }
        Request::Subscribe => Answer::Subscribe,
        Request::Shutdown => {
            stopped = true;
            Answer::Last(Response::Stopping)
        }
    });
    Ok(if stopped {
        "stopped\n".to_owned()
    } else {
        String::new()
    })
}
