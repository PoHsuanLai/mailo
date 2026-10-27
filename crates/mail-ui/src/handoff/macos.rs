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

use super::{Request, inbox_uri, requests_of, uri_of};
use mail_client::profile::ProfileId;
use mail_domain::ThreadId;
use porter_core::AccountId;
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
///
/// The queue outlives the window that reads it: mailo.app goes on with no window open
/// (`crate::background`), and a request that comes then (the Dock icon clicked, a banner's
/// `mailo open`, a `mailto:` link) must still be answered. The window that reads the queue gives
/// it back when it closes ([`Reading`]); a request that finds it given back has the app open a
/// window again ([`Requests::open_with`]), which takes the queue and reads what waited in it.
#[derive(Clone)]
pub struct Requests {
    slot: Arc<Mutex<Slot>>,
    sender: UnboundedSender<Request>,
}

/// The queue, while no window reads it, and how a window is opened to read it.
struct Slot {
    queue: Option<UnboundedReceiver<Request>>,
    /// Opens a window that will read the queue; true when one is on its way.
    open: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// A window is on its way and has not taken the queue yet: the launch's first, or one opened
    /// for a request. Another request meanwhile waits for it rather than opening a second.
    coming: bool,
    /// Whether the Dock icon is heard already: AppKit takes one delegate, set once.
    reopen_heard: bool,
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
            slot: Arc::new(Mutex::new(Slot {
                queue: Some(queue),
                open: None,
                coming: true,
                reopen_heard: false,
            })),
            sender: sender.clone(),
        };
        (requests, sender)
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The queue, for the window that reads it on the thread that draws, until it closes: `None`
    /// while another window reads it.
    pub fn take(&self) -> Option<Reading> {
        let mut slot = self.slot();
        let queue = slot.queue.take()?;
        slot.coming = false;
        Some(Reading {
            queue: Some(queue),
            slot: Arc::clone(&self.slot),
        })
    }

    /// Have `open` open a window to read the queue when a request comes while none does. It
    /// answers whether a window is on its way. Set by the launched app; a test's queue has none,
    /// and a request to it simply waits.
    pub fn open_with(&self, open: impl Fn() -> bool + Send + Sync + 'static) {
        self.slot().open = Some(Arc::new(open));
    }

    /// Queue `request`, and open a window for it when none reads the queue.
    pub(crate) fn send(&self, request: Request) {
        // The receiver lives in this queue's slot or in the window reading it, and goes back to
        // the slot when that window closes: a send fails only once the app is ending.
        let _ = self.sender.send(request);
        self.wanted();
    }

    /// Open a window when the queue waits in the slot with nobody coming for it.
    fn wanted(&self) {
        let open = {
            let mut slot = self.slot();
            if slot.queue.is_none() || slot.coming {
                return;
            }
            let Some(open) = slot.open.clone() else {
                return;
            };
            slot.coming = true;
            open
        };
        if !open() {
            self.slot().coming = false;
        }
    }

    /// Mark the Dock icon heard; false when it was already.
    fn hear_reopen_once(&self) -> bool {
        !std::mem::replace(&mut self.slot().reopen_heard, true)
    }
}

/// The queue as the window that reads it holds it: given back to its [`Requests`] when the
/// window closes, so the next request opens a window again.
pub struct Reading {
    queue: Option<UnboundedReceiver<Request>>,
    slot: Arc<Mutex<Slot>>,
}

impl std::fmt::Debug for Reading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reading(..)")
    }
}

impl Reading {
    /// The next request, or `None` once the app is ending.
    pub async fn recv(&mut self) -> Option<Request> {
        self.queue.as_mut()?.recv().await
    }

    /// The next request already queued, if there is one.
    pub fn try_recv(&mut self) -> Option<Request> {
        self.queue.as_mut()?.try_recv().ok()
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.queue = self.queue.take();
    }
}

/// Where the running window listens: `mailo-<hash>.sock` in the per-user temporary directory,
/// named by this user's state directory. A socket path is limited to about a hundred bytes, which
/// `~/Library/Application Support/mailo/` plus a name can pass (a long account name, a test's
/// scratch home), and `$TMPDIR` on macOS is already the user's own. Hashing the state directory
/// keeps two different homes, a test's and the person's, from sharing one window; and the
/// Profile is hashed in after it, so each Profile's window has a socket of its own. The default
/// Profile's is the one it had before Profiles.
fn socket_path(profile: &ProfileId) -> Option<PathBuf> {
    let state = mail_client::config::state_dir()?;
    Some(std::env::temp_dir().join(socket_name(state.as_os_str().as_bytes(), profile)))
}

/// The socket's file name for the state directory `state` and `profile`.
fn socket_name(state: &[u8], profile: &ProfileId) -> String {
    let mut named = state.to_vec();
    if !profile.is_default() {
        named.push(0);
        named.extend_from_slice(profile.as_str().as_bytes());
    }
    format!("mailo-{:016x}.sock", fnv1a(&named))
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
pub fn serve(profile: &ProfileId) -> Option<Requests> {
    let requests = serve_at(&socket_path(profile)?)?;
    hear_urls(&requests);
    Some(requests)
}

fn serve_at(path: &Path) -> Option<Requests> {
    let listener = bind(path)
        .map_err(|e| eprintln!("opening a conversation from outside is off: {e}"))
        .ok()??;
    let (requests, _sender) = Requests::local();
    let serving = requests.clone();
    std::thread::Builder::new()
        .name("mailo-handoff".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(stream, &serving);
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
fn answer(stream: UnixStream, requests: &Requests) {
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
        requests.send(request);
    }
    let _ = (&stream).write_all(b"ok\n");
}

/// Have the URLs the system opens with mailo reach `requests`' queue. Called while the window's
/// contexts are made, before the event loop: the URL a cold launch carries is delivered when the
/// loop first runs, and waits in the queue until the window reads it.
fn hear_urls(requests: &Requests) {
    let requests = requests.clone();
    let heard = mail_appkit::hear_urls(move |url| {
        for request in requests_of(&[url], None) {
            requests.send(request);
        }
    });
    if !heard {
        eprintln!("a URL opened with mailo is not heard: no Apple event handler");
    }
}

/// Have the Dock icon clicked again raise the window, or open one when none is open. Main thread
/// only, and only once the event loop exists (`mail_appkit::hear_reopen`); false when it could
/// not. Each window that reads the queue asks; the first is heard, and the rest are told it is.
pub fn hear_reopen(requests: &Requests) -> bool {
    if !requests.hear_reopen_once() {
        return true;
    }
    let heard = requests.clone();
    let set = mail_appkit::hear_reopen(move || heard.send(Request::Activate { token: None }));
    if !set {
        // Not heard, so the next window to read the queue asks again.
        requests.slot().reopen_heard = false;
    }
    set
}

/// Ask the running window to open `thread`. True when a window took it; false when none is
/// running, and then the caller opens a window of its own. macOS has no activation token to hand
/// over, so there is none to take.
pub fn deliver(profile: &ProfileId, thread: ThreadId, _token: Option<&str>) -> bool {
    socket_path(profile).is_some_and(|path| send(&path, &uri_of(thread)))
}

/// Ask the running window to open a composer on the `mailto:` link `uri`. True when one took it.
pub fn deliver_mailto(profile: &ProfileId, uri: &str, _token: Option<&str>) -> bool {
    socket_path(profile).is_some_and(|path| send(&path, uri))
}

/// Ask the running window to show `account`'s inbox. True when one took it.
pub fn deliver_inbox(profile: &ProfileId, account: &AccountId, _token: Option<&str>) -> bool {
    socket_path(profile).is_some_and(|path| send(&path, &inbox_uri(account)))
}

/// Ask the running window to raise itself. True when one did; false when none is running.
pub fn deliver_activate(profile: &ProfileId, _token: Option<&str>) -> bool {
    socket_path(profile).is_some_and(|path| send(&path, ACTIVATE))
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
            queue.try_recv(),
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
            queue.try_recv(),
            Some(Request::Mailto { link, .. }) if link.subject == "hi"
        ));
    }

    #[test]
    fn an_accounts_inbox_sent_to_the_socket_reaches_the_window() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("h.sock");
        let requests = serve_at(&path).expect("served");
        let mut queue = requests.take().expect("the queue, once");

        let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
        assert!(send(&path, &inbox_uri(&account)));
        assert_eq!(
            queue.try_recv().ok(),
            Some(Request::Inbox {
                account,
                token: None
            })
        );
    }

    #[test]
    fn the_activate_line_asks_for_the_window_alone() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("h.sock");
        let requests = serve_at(&path).expect("served");
        let mut queue = requests.take().expect("the queue, once");

        assert!(send(&path, ACTIVATE));
        assert_eq!(queue.try_recv(), Some(Request::Activate { token: None }));
    }

    #[test]
    fn a_request_with_no_window_reading_opens_one_and_only_one() {
        let (requests, _sender) = Requests::local();
        let opened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        requests.open_with({
            let opened = Arc::clone(&opened);
            move || {
                opened.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                true
            }
        });
        let count = || opened.load(std::sync::atomic::Ordering::SeqCst);

        // The launch's first window is on its way: a URL that came with the launch waits for it.
        requests.send(Request::Activate { token: None });
        assert_eq!(count(), 0, "the first window is coming");
        let mut first = requests.take().expect("the first window takes the queue");
        assert_eq!(first.try_recv(), Some(Request::Activate { token: None }));

        // While a window reads, a request is its.
        requests.send(Request::Thread {
            thread: thread(),
            token: None,
        });
        assert_eq!(count(), 0);
        assert!(requests.take().is_none(), "one window reads it");

        // The window closes and gives the queue back, with what it had not read.
        drop(first);
        requests.send(Request::Activate { token: None });
        assert_eq!(count(), 1, "the Dock icon opens a window");
        requests.send(Request::Activate { token: None });
        assert_eq!(count(), 1, "a second click waits for the window on its way");

        let mut again = requests.take().expect("the new window takes the queue");
        assert_eq!(
            again.try_recv(),
            Some(Request::Thread {
                thread: thread(),
                token: None
            })
        );
        assert_eq!(again.try_recv(), Some(Request::Activate { token: None }));
        assert_eq!(again.try_recv(), Some(Request::Activate { token: None }));
    }

    #[test]
    fn a_window_that_could_not_open_is_tried_again_at_the_next_request() {
        let (requests, _sender) = Requests::local();
        drop(requests.take());
        let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        requests.open_with({
            let tries = Arc::clone(&tries);
            move || {
                tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                false
            }
        });
        requests.send(Request::Activate { token: None });
        requests.send(Request::Activate { token: None });
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn a_queue_with_no_way_to_open_a_window_keeps_what_it_is_sent() {
        let (requests, _sender) = Requests::local();
        drop(requests.take());
        requests.send(Request::Activate { token: None });
        let mut later = requests.take().expect("given back");
        assert_eq!(later.try_recv(), Some(Request::Activate { token: None }));
    }

    #[test]
    fn the_socket_name_is_stable_per_home_and_short_enough_for_a_socket() {
        assert_eq!(fnv1a(b"/a"), fnv1a(b"/a"));
        assert_ne!(fnv1a(b"/a"), fnv1a(b"/b"));
        // The hash is part of a name two releases must agree on.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        let path = socket_path(&ProfileId::default()).expect("a state directory");
        assert!(path.as_os_str().len() < 100, "{path:?}");
    }

    /// The default Profile keeps the socket it had before Profiles; each other has its own.
    #[test]
    fn each_profile_has_a_socket_of_its_own() {
        let work = ProfileId::try_from("1a2b3c4d".to_owned()).expect("a profile id");
        let default = socket_name(b"/state", &ProfileId::default());
        assert_eq!(default, format!("mailo-{:016x}.sock", fnv1a(b"/state")));
        assert_ne!(socket_name(b"/state", &work), default);
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
