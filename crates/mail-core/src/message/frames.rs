//! What the reader has already rendered, kept so opening a conversation twice costs a lookup.
//!
//! [`render`] is a pure function of the message's raw blob and the sanitize policy: a blob is
//! content-addressed, so the same [`BlobId`] is the same bytes for ever, and the cache can never
//! need invalidating. The whole [`SanitizePolicy`] is in the key, version included — a key
//! without it would serve markup sanitized under one policy to a reader that asked for another,
//! which for `RemoteImages` is a read receipt nobody granted.
//!
//! Bounded by the size of the html it holds, least recently used out first. It is a `Mutex`, not
//! a signal, so [`Frames::warm`] can fill it from an ordinary thread that nothing has to poll
//! (F140). The cache is a value the front-end owns and hands to whoever renders, not a process
//! global: two of them share nothing.

use super::frame::{FrameBody, Source, render};
use mail_domain::{BlobId, Message, ThreadId};
use mail_mime::{Parsed, SanitizePolicy};
use mail_store::{SqliteStore, Store};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// How much rendered html to keep, in bytes. A long newsletter is a few hundred kilobytes.
const BUDGET: usize = 32 * 1024 * 1024;

/// What a rendering is cached under.
pub type Key = (BlobId, SanitizePolicy);

/// The body OpenPGP or S/MIME opened a message to, when it has been opened. Such a message is
/// not cached: its rendering is a function of more than the blob, and of a key that may be
/// locked again.
pub type Unsealed = Arc<dyn Fn(&Message) -> Option<Parsed> + Send + Sync>;

struct Entry {
    body: FrameBody,
    bytes: usize,
    /// When it was last put or read, by [`Cache::tick`]: the smallest goes first.
    used: u64,
}

/// Hashed, so a lookup on the thread that draws costs the same with ten thousand small bodies in
/// it as with ten. Eviction is the one walk over it, and only when a put goes over the budget.
struct Cache {
    entries: HashMap<Key, Entry>,
    held: usize,
    budget: usize,
    tick: u64,
}

impl Cache {
    fn new(budget: usize) -> Self {
        Cache {
            entries: HashMap::new(),
            held: 0,
            budget,
            tick: 0,
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&mut self, key: Key) -> Option<FrameBody> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.get_mut(&key)?;
        entry.used = tick;
        Some(entry.body.clone())
    }

    fn put(&mut self, key: Key, body: FrameBody) {
        self.tick += 1;
        let used = self.tick;
        if let Some(gone) = self.entries.remove(&key) {
            self.held -= gone.bytes;
        }
        let bytes = size(&body);
        // One body larger than the whole budget is not worth evicting everything for.
        if bytes > self.budget {
            return;
        }
        self.held += bytes;
        self.entries.insert(key, Entry { body, bytes, used });
        while self.held > self.budget {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key);
            let Some(gone) = oldest.and_then(|key| self.entries.remove(&key)) else {
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

/// Raw mail, in bytes, that the reader renders on the frame that draws it. A conversation nobody
/// rendered ahead (one opened from a search, far down the list) would otherwise hold that frame
/// for every parse and sanitize in it; what is past this renders on a blocking thread instead,
/// and is drawn when it lands. A plain reply is a few kilobytes, a newsletter a few hundred.
pub const ON_THE_FRAME: u64 = 128 * 1024;

/// How many conversations from the top of the list are rendered before anyone opens one.
pub const FIRST_SCREEN: usize = 20;

/// Where each rendering one reader sent off the thread has got to.
#[derive(Default)]
pub struct Sent {
    /// Being rendered: drawn as a placeholder until it lands.
    going: Vec<Key>,
    /// Rendered, and still not in the cache — it was larger than the whole budget, or evicted
    /// since. Rendered on the frame from then on, or it would be sent for ever.
    landed: Vec<Key>,
}

impl Sent {
    /// `keys` are being rendered off the thread.
    pub fn start(&mut self, keys: &[Key]) {
        self.going.extend(keys.iter().copied());
    }

    /// `keys` have been rendered.
    pub fn finish(&mut self, keys: Vec<Key>) {
        self.going.retain(|key| !keys.contains(key));
        self.landed.extend(keys);
    }
}

/// The reader's rendered bodies, and the thread that renders them ahead.
pub struct Frames {
    unsealed: Unsealed,
    cache: Mutex<Cache>,
    /// Bumped by every [`Frames::warm`], so that only the newest one keeps going.
    warming: AtomicU64,
}

impl Frames {
    /// An empty cache. `unsealed` says which messages were opened to a body of their own.
    pub fn new(unsealed: Unsealed) -> Self {
        Self::with_budget(unsealed, BUDGET)
    }

    /// A cache for messages that are never sealed.
    pub fn plain() -> Self {
        Self::new(Arc::new(|_| None))
    }

    fn with_budget(unsealed: Unsealed, budget: usize) -> Self {
        Frames {
            unsealed,
            cache: Mutex::new(Cache::new(budget)),
            warming: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// What `message` renders to under `policy`, every time, past the cache. For a measurement
    /// that must not hit it.
    pub fn uncached(
        &self,
        store: &SqliteStore,
        message: &Message,
        policy: SanitizePolicy,
    ) -> FrameBody {
        render(store, message, policy, &*self.unsealed).0
    }

    /// What `message` renders to under `policy`, from the cache when it has been rendered before.
    ///
    /// A message with no body yet, or one opened to a body of its own by OpenPGP or S/MIME, is
    /// not cached: the first has nothing to key on and the second is a function of more than the
    /// blob.
    pub fn rendered(
        &self,
        store: &SqliteStore,
        message: &Message,
        policy: SanitizePolicy,
    ) -> FrameBody {
        let Some(key) = self.keyed(message, policy) else {
            return self.uncached(store, message, policy);
        };
        if let Some(had) = self.lock().get(key) {
            return had;
        }
        // Rendered outside the lock: a parse and a sanitize must not hold up a lookup. Kept only
        // when it was rendered from the blob: a seal opened since `keyed` looked would otherwise
        // leave plaintext under the blob's key, served once the key is locked again.
        let (body, source) = render(store, message, policy, &*self.unsealed);
        if source == Source::Stored {
            self.lock().put(key, body.clone());
        }
        body
    }

    /// The cached rendering, if there is one. Never renders.
    pub fn peek(&self, message: &Message, policy: SanitizePolicy) -> Option<FrameBody> {
        self.lock().get(self.keyed(message, policy)?)
    }

    /// The key `message` is cached under at `policy`, when it can be.
    pub fn keyed(&self, message: &Message, policy: SanitizePolicy) -> Option<Key> {
        if (self.unsealed)(message).is_some() {
            return None;
        }
        Some((message.body.raw()?, policy))
    }

    /// What `message` renders to under `policy`, if it can be had on this frame: from the cache,
    /// or rendered here while `room` still holds its raw size. `None` when it is to be rendered
    /// off the thread; it is then in `later`, unless it is already being rendered.
    pub fn on_the_frame(
        &self,
        store: &SqliteStore,
        message: &Message,
        policy: SanitizePolicy,
        room: &mut u64,
        sent: &Sent,
        later: &mut Vec<(Message, SanitizePolicy)>,
    ) -> Option<FrameBody> {
        let Some(key) = self.keyed(message, policy) else {
            return Some(self.rendered(store, message, policy));
        };
        if let Some(had) = self.lock().get(key) {
            return Some(had);
        }
        if sent.going.contains(&key) {
            return None;
        }
        let size = store.blobs().size(key.0).unwrap_or(0);
        if size <= *room || sent.landed.contains(&key) {
            *room = room.saturating_sub(size);
            return Some(self.rendered(store, message, policy));
        }
        if !later
            .iter()
            .any(|(m, p)| m.id == message.id && *p == policy)
        {
            later.push((message.clone(), policy));
        }
        None
    }

    /// Render the messages of `threads` into the cache on a thread of its own, in order, so that
    /// opening any of them is a lookup.
    ///
    /// A plain `std::thread`, not a task: it writes into a `Mutex` and needs nothing to poll it
    /// and nothing to notice when it finishes, which is why it works whatever the window's event
    /// loop is doing. Each call supersedes the one before: a selection that has moved on
    /// abandons what it was warming between messages rather than finishing it. The store is read
    /// on that thread too, so asking costs the caller nothing.
    pub fn warm(
        self: &Arc<Self>,
        store: Arc<SqliteStore>,
        threads: Vec<ThreadId>,
        policy: SanitizePolicy,
    ) {
        let generation = self.warming.fetch_add(1, Ordering::Relaxed) + 1;
        let frames = Arc::clone(self);
        let current = move || frames.warming.load(Ordering::Relaxed) == generation;
        let frames = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("reader-warm".to_owned())
            .spawn(move || frames.warm_now(&store, &threads, policy, current));
    }

    /// [`Frames::warm`]'s work, on the calling thread. `still_wanted` is asked before each
    /// message.
    pub fn warm_now(
        &self,
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
                if self.peek(&message, policy).is_none() {
                    let _ = self.rendered(store, &message, policy);
                }
            }
        }
    }
}

/// Which conversations to render ahead, most wanted first: the two after the open one and the
/// one before it — where `j` and `k` go next — then the top of the list. Never the open one,
/// which the reader is rendering already, and never one twice.
pub fn ahead(open: Option<ThreadId>, list: &[ThreadId], screen: usize) -> Vec<ThreadId> {
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

    #[test]
    fn putting_a_key_twice_does_not_count_it_twice() {
        let mut cache = Cache::new(100);
        let k = key(1, RemoteImages::Blocked, 1);
        cache.put(k, html(60));
        cache.put(k, html(60));
        assert_eq!(cache.held, 60);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn two_caches_share_nothing() {
        let (one, two) = (Frames::plain(), Frames::plain());
        let k = key(1, RemoteImages::Blocked, 1);
        one.lock().put(k, html(10));
        assert!(one.lock().get(k).is_some());
        assert!(two.lock().get(k).is_none());
    }

    fn ids(n: u128) -> Vec<ThreadId> {
        (1..=n)
            .map(|i| ThreadId::from_uuid(uuid::Uuid::from_u128(i)))
            .collect()
    }

    /// Which conversations are warmed ahead, by their place in the list: the open one's
    /// neighbours first, then the top of the list, never the open one, and nothing past either
    /// end.
    #[test]
    fn ahead_order() {
        type Row = (&'static str, u128, Option<usize>, usize, &'static [usize]);
        const CASES: &[Row] = &[
            ("neighbours, then the top", 6, Some(2), 3, &[3, 4, 1, 0]),
            ("nothing open is the top", 6, None, 3, &[0, 1, 2]),
            ("open at the last row", 3, Some(2), 0, &[1]),
            ("open at the first row", 3, Some(0), 0, &[1, 2]),
        ];
        for (name, rows, open, screen, want) in CASES {
            let list = ids(*rows);
            let want: Vec<ThreadId> = want.iter().map(|row| list[*row]).collect();
            assert_eq!(
                ahead(open.map(|row| list[row]), &list, *screen),
                want,
                "{name}"
            );
        }
    }
}
