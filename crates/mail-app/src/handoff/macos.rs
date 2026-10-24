//! macOS's half of [`super`]: no session bus, so the running window listens on a Unix socket in
//! the user's temporary directory, and AppKit's own events (a URL opened, the Dock icon clicked again)
//! reach it through the application delegate in `mail-appkit`.
//!
//! Two ways in, one queue. `mailo open <thread>` from a terminal or a banner's helper is a second
//! process: [`deliver`] writes the conversation's URI to the socket and waits for the window's
//! `ok`. The system's own launch of an app that is already running is not a second process at all:
//! LaunchServices tells the one running, and [`hear_urls`] and [`hear_reopen`] turn what it says into the same requests.
//! The socket is also how a second `mailo` with nothing to do ([`deliver_activate`]) raises the
//! window instead of opening another.
//!
//! One line per request: `mailo:thread/<id>`, or `mailo:activate` for the window alone. A path the
//! socket cannot have (too long for `sockaddr_un`, no home) means no hand-off, as no bus does on
//! Linux.

use super::{Request, requests_of, uri_of};
use mail_domain::ThreadId;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The request that asks for the window alone.
const ACTIVATE: &str = "mailo:activate";

/// How long either end waits on the other before giving up: both are on this machine.
const PATIENCE: Duration = Duration::from_secs(1);

/// No request is longer than a URI of ours; a client that sends more is not one.
const LONGEST_LINE: u64 = 512;

/// What the window reads requests from, as a root context.
#[derive(Clone)]
pub struct Requests {
    queue: Arc<Mutex<Option<UnboundedReceiver<Request>>>>,
    sender: UnboundedSender<Request>,
}

impl std::fmt::Debug for Requests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Requests(..)")
    }
}

impl Requests {
    /// A queue with nobody serving it, for a test to push into.
    pub fn local() -> (Requests, UnboundedSender<Request>) {
        let (sender, queue) = unbounded_channel();
        let requests = Requests {
            queue: Arc::new(Mutex::new(Some(queue))),
            sender: sender.clone(),
        };
        (requests, sender)
    }

    /// The queue, once: the window reads it on the thread that draws.
    pub fn take(&self) -> Option<UnboundedReceiver<Request>> {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Another way in to the same queue, for the application delegate.
    pub(crate) fn sender(&self) -> UnboundedSender<Request> {
        self.sender.clone()
    }
}

/// Where the running window listens: `mailo-<hash>.sock` in the per-user temporary directory,
/// named by this user's state directory. A socket path is limited to about a hundred bytes, which
/// `~/Library/Application Support/mailo/` plus a name can pass (a long account name, a test's
/// scratch home), and `$TMPDIR` on macOS is already the user's own. Hashing the state directory
/// keeps two different homes, a test's and the person's, from sharing one window.
fn socket_path() -> Option<PathBuf> {
    let state = mail_core::config::state_dir()?;
    Some(std::env::temp_dir().join(format!(
        "mailo-{:016x}.sock",
        fnv1a(state.as_os_str().as_bytes())
    )))
}

/// FNV-1a: a hash that never changes between releases, since an old window and a new `mailo open`
/// must find the same socket.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Start answering on the socket, or `None` when there is no place for one or another window
/// already holds it (this one then simply is not handed to).
pub fn serve() -> Option<Requests> {
    let requests = serve_at(&socket_path()?)?;
    hear_urls(&requests);
    Some(requests)
}

fn serve_at(path: &Path) -> Option<Requests> {
    let listener = bind(path)
        .map_err(|e| eprintln!("opening a conversation from outside is off: {e}"))
        .ok()??;
    let (requests, sender) = Requests::local();
    std::thread::Builder::new()
        .name("mailo-handoff".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(stream, &sender);
            }
        })
        .map_err(|e| eprintln!("opening a conversation from outside is off: {e}"))
        .ok()?;
    Some(requests)
}

/// The listener on `path`; `Ok(None)` when a live window already answers there. A socket file
/// nobody answers on (a window that was killed) is replaced.
fn bind(path: &Path) -> std::io::Result<Option<UnixListener>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if UnixStream::connect(path).is_ok() {
        return Ok(None);
    }
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(Some(listener))
}

/// Read one request line, queue what it asks, and say `ok`.
fn answer(stream: UnixStream, sender: &UnboundedSender<Request>) {
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let _ = stream.set_write_timeout(Some(PATIENCE));
    let mut line = String::new();
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    if BufReader::new(reader.take(LONGEST_LINE))
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    let uri = line.trim_end().to_owned();
    for request in requests_of(&[uri], None) {
        let _ = sender.send(request);
    }
    let _ = (&stream).write_all(b"ok\n");
}

/// Have the URLs the system opens with mailo reach `requests`' queue. Called while the window's
/// contexts are made, before the event loop: the URL a cold launch carries is delivered when the
/// loop first runs, and waits in the queue until the window reads it.
fn hear_urls(requests: &Requests) {
    let sender = requests.sender();
    let heard = mail_appkit::hear_urls(move |url| {
        for request in requests_of(&[url], None) {
            let _ = sender.send(request);
        }
    });
    if !heard {
        eprintln!("a URL opened with mailo is not heard: no Apple event handler");
    }
}

/// Have the Dock icon clicked again raise the window. Main thread only, and only once the event
/// loop exists (`mail_appkit::hear_reopen`); false when it could not.
pub fn hear_reopen(requests: &Requests) -> bool {
    let sender = requests.sender();
    mail_appkit::hear_reopen(move || {
        let _ = sender.send(Request::Activate { token: None });
    })
}

/// Ask the running window to open `thread`. True when a window took it; false when none is
/// running, and then the caller opens a window of its own. macOS has no activation token to hand
/// over, so there is none to take.
pub fn deliver(thread: ThreadId, _token: Option<&str>) -> bool {
    socket_path().is_some_and(|path| send(&path, &uri_of(thread)))
}

/// Ask the running window to open a composer on the `mailto:` link `uri`. True when one took it.
pub fn deliver_mailto(uri: &str, _token: Option<&str>) -> bool {
    socket_path().is_some_and(|path| send(&path, uri))
}

/// Ask the running window to raise itself. True when one did; false when none is running.
pub fn deliver_activate(_token: Option<&str>) -> bool {
    socket_path().is_some_and(|path| send(&path, ACTIVATE))
}

fn send(path: &Path, uri: &str) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let _ = stream.set_write_timeout(Some(PATIENCE));
    if stream.write_all(format!("{uri}\n").as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply == "ok\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread() -> ThreadId {
        ThreadId::from_uuid(uuid::Uuid::from_u128(0x1234))
    }

    #[test]
    fn a_conversation_sent_to_the_socket_reaches_the_window_and_is_acknowledged() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("h.sock");
        let requests = serve_at(&path).expect("served");
        let mut queue = requests.take().expect("the queue, once");

        assert!(send(&path, &uri_of(thread())));
        assert_eq!(
            queue.try_recv().ok(),
            Some(Request::Thread {
                thread: thread(),
                token: None
            })
        );
    }

    #[test]
    fn a_mailto_link_sent_to_the_socket_reaches_the_window() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("h.sock");
        let requests = serve_at(&path).expect("served");
        let mut queue = requests.take().expect("the queue, once");

        assert!(send(&path, "mailto:a@example.test?subject=hi"));
        assert!(matches!(
            queue.try_recv().ok(),
            Some(Request::Mailto { link, .. }) if link.subject == "hi"
        ));
    }

    #[test]
    fn the_activate_line_asks_for_the_window_alone() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("h.sock");
        let requests = serve_at(&path).expect("served");
        let mut queue = requests.take().expect("the queue, once");

        assert!(send(&path, ACTIVATE));
        assert_eq!(
            queue.try_recv().ok(),
            Some(Request::Activate { token: None })
        );
    }

    #[test]
    fn the_socket_name_is_stable_per_home_and_short_enough_for_a_socket() {
        assert_eq!(fnv1a(b"/a"), fnv1a(b"/a"));
        assert_ne!(fnv1a(b"/a"), fnv1a(b"/b"));
        // The hash is part of a name two releases must agree on.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        let path = socket_path().expect("a state directory");
        assert!(path.as_os_str().len() < 100, "{path:?}");
    }

    #[test]
    fn nothing_listening_is_no_delivery() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        assert!(!send(&dir.path().join("none.sock"), &uri_of(thread())));
    }

    #[test]
    fn a_second_window_does_not_take_the_socket_and_a_dead_one_is_replaced() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("h.sock");
        let first = serve_at(&path).expect("the first window serves");
        assert!(serve_at(&path).is_none(), "a live window keeps it");

        // A window that was killed leaves the file and nobody behind it.
        drop(first);
        let stale = dir.path().join("stale.sock");
        drop(UnixListener::bind(&stale).expect("a socket"));
        assert!(stale.exists());
        assert!(serve_at(&stale).is_some(), "a dead socket is replaced");
    }
}
