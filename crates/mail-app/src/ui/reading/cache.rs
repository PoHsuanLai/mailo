//! What the reader has already rendered, kept so opening a conversation twice costs a lookup.
//!
//! `render_message` is a pure function of the message's raw blob and the sanitize policy: a blob
//! is content-addressed, so the same [`BlobId`] is the same bytes for ever, and the cache can
//! never need invalidating. The whole [`SanitizePolicy`] is in the key, version included — a key
//! without it would serve markup sanitized under one policy to a reader that asked for another,
//! which for `RemoteImages` is a read receipt nobody granted.
//!
//! Bounded by the size of the html it holds, least recently used out first. It is a `Mutex`, not
//! a signal, so [`warm`] can fill it from an ordinary thread that nothing has to poll (F140).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dioxus::prelude::*;
use mail_domain::{BlobId, Message, ThreadId, ThreadSummary};
use mail_mime::SanitizePolicy;
use mail_store::{SqliteStore, Store};

use super::{FrameBody, Source, render, render_message_uncached};
use crate::ui::view::Shell;

/// How much rendered html to keep, in bytes. A long newsletter is a few hundred kilobytes.
const BUDGET: usize = 32 * 1024 * 1024;

pub(super) type Key = (BlobId, SanitizePolicy);

struct Entry {
    body: FrameBody,
    bytes: usize,
    /// When it was last put or read, by [`Cache::tick`]: the smallest goes first.
    used: u64,
}

/// Hashed, so a lookup on the thread that draws costs the same with ten thousand small bodies in
/// it as with ten. Eviction is the one walk over it, and only when a put goes over the budget.
struct Cache {
    entries: Option<HashMap<Key, Entry>>,
    held: usize,
    budget: usize,
    tick: u64,
}

impl Cache {
    const fn new(budget: usize) -> Self {
        Cache {
            entries: None,
            held: 0,
            budget,
            tick: 0,
        }
    }

    fn entries(&mut self) -> &mut HashMap<Key, Entry> {
        self.entries.get_or_insert_with(HashMap::new)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.as_ref().map_or(0, HashMap::len)
    }

    fn get(&mut self, key: Key) -> Option<FrameBody> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries().get_mut(&key)?;
        entry.used = tick;
        Some(entry.body.clone())
    }

    fn put(&mut self, key: Key, body: FrameBody) {
        self.tick += 1;
        let used = self.tick;
        if let Some(gone) = self.entries().remove(&key) {
            self.held -= gone.bytes;
        }
        let bytes = size(&body);
        // One body larger than the whole budget is not worth evicting everything for.
        if bytes > self.budget {
            return;
        }
        self.held += bytes;
        self.entries().insert(key, Entry { body, bytes, used });
        while self.held > self.budget {
            let oldest = self
                .entries()
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key);
            let Some(gone) = oldest.and_then(|key| self.entries().remove(&key)) else {
                break;
            };
            self.held -= gone.bytes;
        }
    }
}

fn size(body: &FrameBody) -> usize {
    match body {
        FrameBody::NotFetched => 0,
        FrameBody::Present { html, fetches, .. } => {
            html.len() + fetches.iter().map(String::len).sum::<usize>()
        }
    }
}

static CACHE: Mutex<Cache> = Mutex::new(Cache::new(BUDGET));

fn lock() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|held| held.into_inner())
}

/// What `message` renders to under `policy`, from the cache when it has been rendered before.
///
/// A message with no body yet, or one opened to a body of its own by OpenPGP or S/MIME, is not
/// cached: the first has nothing to key on and the second is a function of more than the blob.
pub fn rendered(store: &SqliteStore, message: &Message, policy: SanitizePolicy) -> FrameBody {
    let Some(key) = keyed(message, policy) else {
        return render_message_uncached(store, message, policy);
    };
    if let Some(had) = lock().get(key) {
        return had;
    }
    // Rendered outside the lock: a parse and a sanitize must not hold up a lookup. Kept only
    // when it was rendered from the blob: a seal opened since `keyed` looked would otherwise
    // leave plaintext under the blob's key, served once the key is locked again.
    let (body, source) = render(store, message, policy);
    if source == Source::Stored {
        lock().put(key, body.clone());
    }
    body
}

/// The cached rendering, if there is one. Never renders.
pub(super) fn peek(message: &Message, policy: SanitizePolicy) -> Option<FrameBody> {
    lock().get(keyed(message, policy)?)
}

/// Raw mail, in bytes, that the reader renders on the frame that draws it. A conversation nobody
/// rendered ahead (one opened from a search, far down the list) would otherwise hold that frame
/// for every parse and sanitize in it; what is past this renders on a blocking thread instead,
/// and is drawn when it lands. A plain reply is a few kilobytes, a newsletter a few hundred.
pub(super) const ON_THE_FRAME: u64 = 128 * 1024;

/// Where each rendering one reader sent off the thread has got to.
#[derive(Default)]
pub(super) struct Sent {
    /// Being rendered: drawn as a placeholder until it lands.
    going: Vec<Key>,
    /// Rendered, and still not in the cache — it was larger than the whole budget, or evicted
    /// since. Rendered on the frame from then on, or it would be sent for ever.
    landed: Vec<Key>,
}

/// What `message` renders to under `policy`, if it can be had on this frame: from the cache, or
/// rendered here while `room` still holds its raw size. `None` when it is to be rendered off the
/// thread; it is then in `later`, unless it is already being rendered.
pub(super) fn on_the_frame(
    store: &SqliteStore,
    message: &Message,
    policy: SanitizePolicy,
    room: &mut u64,
    sent: &Sent,
    later: &mut Vec<(Message, SanitizePolicy)>,
) -> Option<FrameBody> {
    let Some(key) = keyed(message, policy) else {
        return Some(rendered(store, message, policy));
    };
    if let Some(had) = lock().get(key) {
        return Some(had);
    }
    if sent.going.contains(&key) {
        return None;
    }
    let size = store.blobs().size(&store.reader(), key.0).unwrap_or(0);
    if size <= *room || sent.landed.contains(&key) {
        *room = room.saturating_sub(size);
        return Some(rendered(store, message, policy));
    }
    if !later
        .iter()
        .any(|(m, p)| m.id == message.id && *p == policy)
    {
        later.push((message.clone(), policy));
    }
    None
}

/// Render `later` on a blocking thread, then move `landed` so the reader draws them.
pub(super) fn render_later(
    store: Arc<SqliteStore>,
    later: Vec<(Message, SanitizePolicy)>,
    sent: std::rc::Rc<std::cell::RefCell<Sent>>,
    mut landed: Signal<u64>,
) {
    let keys: Vec<Key> = later
        .iter()
        .filter_map(|(message, policy)| keyed(message, *policy))
        .collect();
    sent.borrow_mut().going.extend(keys.iter().copied());
    spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            for (message, policy) in &later {
                let _ = rendered(&store, message, *policy);
            }
        })
        .await;
        {
            let mut sent = sent.borrow_mut();
            sent.going.retain(|key| !keys.contains(key));
            sent.landed.extend(keys);
        }
        landed += 1;
    });
}

fn keyed(message: &Message, policy: SanitizePolicy) -> Option<Key> {
    if super::super::pgp::parsed(message).is_some() {
        return None;
    }
    Some((message.body.raw()?, policy))
}

/// How many conversations from the top of the list are rendered before anyone opens one.
pub(in crate::ui) const FIRST_SCREEN: usize = 20;

/// Which conversations to render ahead, most wanted first: the two after the open one and the
/// one before it — where `j` and `k` go next — then the top of the list. Never the open one,
/// which the reader is rendering already, and never one twice.
pub(in crate::ui) fn ahead(
    open: Option<ThreadId>,
    list: &[ThreadId],
    screen: usize,
) -> Vec<ThreadId> {
    let mut order = Vec::new();
    if let Some(here) = open.and_then(|id| list.iter().position(|it| *it == id)) {
        let neighbours = [
            here.checked_add(1),
            here.checked_add(2),
            here.checked_sub(1),
        ];
        order.extend(neighbours.into_iter().flatten().filter_map(|i| list.get(i)));
    }
    for id in list.iter().take(screen) {
        if !order.contains(id) {
            order.push(*id);
        }
    }
    order.retain(|id| Some(*id) != open);
    order
}

/// Bumped by every [`warm`], so that only the newest one keeps going.
static WARMING: AtomicU64 = AtomicU64::new(0);

/// Render the messages of `threads` into the cache on a thread of its own, in order, so that
/// opening any of them is a lookup.
///
/// A plain `std::thread`, not a task: it writes into a `Mutex` and needs nothing to poll it and
/// nothing to notice when it finishes, which is why it works whatever the window's event loop is
/// doing. Each call supersedes the one before: a selection that has moved on abandons what it
/// was warming between messages rather than finishing it. The store is read on that thread too,
/// so asking costs the caller nothing.
pub fn warm(store: Arc<SqliteStore>, threads: Vec<ThreadId>, policy: SanitizePolicy) {
    let generation = WARMING.fetch_add(1, Ordering::Relaxed) + 1;
    let current = move || WARMING.load(Ordering::Relaxed) == generation;
    let _ = std::thread::Builder::new()
        .name("reader-warm".to_owned())
        .spawn(move || warm_now(&store, &threads, policy, current));
}

/// [`warm`]'s work, on the calling thread. `still_wanted` is asked before each message.
fn warm_now(
    store: &SqliteStore,
    threads: &[ThreadId],
    policy: SanitizePolicy,
    still_wanted: impl Fn() -> bool,
) {
    for thread in threads {
        let Ok(loaded) = store.thread(*thread) else {
            continue;
        };
        for id in &loaded.messages {
            if !still_wanted() {
                return;
            }
            let Ok(message) = store.message(*id) else {
                continue;
            };
            if peek(&message, policy).is_none() {
                let _ = rendered(store, &message, policy);
            }
        }
    }
}

/// Keep what the person is about to open rendered: the top of the list once it is drawn, and
/// the neighbours of the open conversation whenever it changes. Their bodies, where they are not
/// here yet, are what the next body pass fetches first (`mail_runtime::wanted`).
///
/// Warmed under the frame's policy, which is the one a conversation opens with: showing remote
/// images is asked for per conversation, and opening another one clears it.
pub(in crate::ui) fn use_warming(shell: Signal<Shell>, threads: Memo<Vec<ThreadSummary>>) {
    let store = use_context::<Arc<SqliteStore>>();
    // A memo, so that only a change of the open conversation counts, not every keystroke the
    // shell sees.
    let open = use_memo(move || shell.read().open);
    use_effect(move || {
        let list: Vec<ThreadId> = threads.read().iter().map(|thread| thread.id).collect();
        let order = ahead(open(), &list, FIRST_SCREEN);
        // The same order is what a body pass fetches first, with the open conversation ahead of
        // it: a message without its body yet cannot be rendered ahead, only fetched ahead.
        let bodies: Vec<ThreadId> = open().into_iter().chain(order.iter().copied()).collect();
        mail_runtime::wanted::ask_first(&bodies);
        warm(store.clone(), order, SanitizePolicy::FRAME);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_mime::RemoteImages;

    fn html(n: usize) -> FrameBody {
        FrameBody::Present {
            html: "x".repeat(n),
            blocked_remote: false,
            fetches: Vec::new(),
        }
    }

    fn key(n: u8, remote_images: RemoteImages, version: u32) -> Key {
        (
            BlobId::from_uuid(uuid::Uuid::from_u128(u128::from(n))),
            SanitizePolicy {
                remote_images,
                version,
                ..SanitizePolicy::FRAME
            },
        )
    }

    #[test]
    fn a_hit_is_the_body_that_was_put() {
        let mut cache = Cache::new(1000);
        let k = key(1, RemoteImages::Blocked, 1);
        cache.put(k, html(10));
        assert_eq!(cache.get(k), Some(html(10)));
    }

    #[test]
    fn the_policy_is_part_of_the_key() {
        let mut cache = Cache::new(1000);
        cache.put(key(1, RemoteImages::Blocked, 1), html(10));
        // The same bytes under another policy, or the same policy at another version, is a miss.
        assert_eq!(cache.get(key(1, RemoteImages::Allowed, 1)), None);
        assert_eq!(cache.get(key(1, RemoteImages::Blocked, 2)), None);
    }

    #[test]
    fn it_stays_under_its_budget_and_drops_the_least_recently_used() {
        let mut cache = Cache::new(100);
        let (a, b, c) = (
            key(1, RemoteImages::Blocked, 1),
            key(2, RemoteImages::Blocked, 1),
            key(3, RemoteImages::Blocked, 1),
        );
        cache.put(a, html(40));
        cache.put(b, html(40));
        // `a` is read again, so `b` is now the oldest.
        assert!(cache.get(a).is_some());
        cache.put(c, html(40));
        assert!(cache.held <= 100);
        assert!(cache.get(b).is_none(), "the oldest went first");
        assert!(cache.get(a).is_some() && cache.get(c).is_some());
    }

    #[test]
    fn a_body_larger_than_the_budget_is_not_kept_and_evicts_nothing() {
        let mut cache = Cache::new(100);
        let small = key(1, RemoteImages::Blocked, 1);
        cache.put(small, html(10));
        cache.put(key(2, RemoteImages::Blocked, 1), html(500));
        assert!(cache.get(small).is_some());
        assert_eq!(cache.len(), 1);
    }

    fn ids(n: u128) -> Vec<ThreadId> {
        (1..=n)
            .map(|i| ThreadId::from_uuid(uuid::Uuid::from_u128(i)))
            .collect()
    }

    #[test]
    fn ahead_is_the_neighbours_first_then_the_top_and_never_the_open_one() {
        let list = ids(6);
        let order = ahead(Some(list[2]), &list, 3);
        assert_eq!(order, vec![list[3], list[4], list[1], list[0]]);
    }

    #[test]
    fn ahead_with_nothing_open_is_the_top_of_the_list() {
        let list = ids(6);
        assert_eq!(ahead(None, &list, 3), list[..3].to_vec());
    }

    #[test]
    fn ahead_at_either_end_does_not_reach_past_it() {
        let list = ids(3);
        assert_eq!(ahead(Some(list[2]), &list, 0), vec![list[1]]);
        assert_eq!(ahead(Some(list[0]), &list, 0), vec![list[1], list[2]]);
    }

    #[test]
    fn warming_a_conversation_makes_opening_it_a_lookup() {
        let (store, _dir) = crate::ui::fixtures::realistic();
        let thread = crate::ui::fixtures::thread_like(&store, "rust-lang/rust");
        let messages: Vec<Message> = store
            .thread(thread)
            .unwrap()
            .messages
            .iter()
            .map(|id| store.message(*id).unwrap())
            .collect();
        let policy = SanitizePolicy::FRAME;
        assert!(
            messages.iter().any(|m| m.body.raw().is_some()),
            "the fixture has bodies"
        );
        warm_now(&store, &[thread], policy, || true);
        for message in messages.iter().filter(|m| m.body.raw().is_some()) {
            let had = peek(message, policy).expect("warmed");
            assert_eq!(had, render_message_uncached(&store, message, policy));
        }
    }

    #[test]
    fn warming_that_is_no_longer_wanted_stops() {
        let (store, _dir) = crate::ui::fixtures::realistic();
        let thread = crate::ui::fixtures::thread_like(&store, "rust-lang/rust");
        let message = store
            .message(store.thread(thread).unwrap().messages[0])
            .unwrap();
        // A policy no other test warms with, so the shared cache cannot already hold it.
        let policy = SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            version: u32::MAX,
            ..SanitizePolicy::FRAME
        };
        warm_now(&store, &[thread], policy, || false);
        assert_eq!(peek(&message, policy), None);
    }

    #[test]
    fn putting_a_key_twice_does_not_count_it_twice() {
        let mut cache = Cache::new(100);
        let k = key(1, RemoteImages::Blocked, 1);
        cache.put(k, html(60));
        cache.put(k, html(60));
        assert_eq!(cache.held, 60);
        assert_eq!(cache.len(), 1);
    }
}
