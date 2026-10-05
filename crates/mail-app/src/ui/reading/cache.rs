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

use std::sync::Mutex;

use mail_domain::{BlobId, Message};
use mail_mime::SanitizePolicy;
use mail_store::SqliteStore;

use super::{FrameBody, render_message_uncached};

/// How much rendered html to keep, in bytes. A long newsletter is a few hundred kilobytes.
const BUDGET: usize = 32 * 1024 * 1024;

type Key = (BlobId, SanitizePolicy);

struct Entry {
    key: Key,
    body: FrameBody,
    bytes: usize,
}

/// Oldest first: a hit moves its entry to the back, and eviction takes from the front. A `Vec`
/// scanned linearly, because a policy is not `Hash` and a window holds a few hundred of these.
struct Cache {
    entries: Vec<Entry>,
    held: usize,
    budget: usize,
}

impl Cache {
    const fn new(budget: usize) -> Self {
        Cache {
            entries: Vec::new(),
            held: 0,
            budget,
        }
    }

    fn get(&mut self, key: Key) -> Option<FrameBody> {
        let at = self.entries.iter().position(|entry| entry.key == key)?;
        let entry = self.entries.remove(at);
        let body = entry.body.clone();
        self.entries.push(entry);
        Some(body)
    }

    fn put(&mut self, key: Key, body: FrameBody) {
        if let Some(at) = self.entries.iter().position(|entry| entry.key == key) {
            self.held -= self.entries.remove(at).bytes;
        }
        let bytes = size(&body);
        // One body larger than the whole budget is not worth evicting everything for.
        if bytes > self.budget {
            return;
        }
        self.held += bytes;
        self.entries.push(Entry { key, body, bytes });
        while self.held > self.budget && !self.entries.is_empty() {
            self.held -= self.entries.remove(0).bytes;
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
    // Rendered outside the lock: a parse and a sanitize must not hold up a lookup.
    let body = render_message_uncached(store, message, policy);
    lock().put(key, body.clone());
    body
}

/// The cached rendering, if there is one. Never renders.
pub(super) fn peek(message: &Message, policy: SanitizePolicy) -> Option<FrameBody> {
    lock().get(keyed(message, policy)?)
}

fn keyed(message: &Message, policy: SanitizePolicy) -> Option<Key> {
    if super::super::pgp::parsed(message).is_some() {
        return None;
    }
    Some((message.body.raw()?, policy))
}

/// Render `messages` into the cache on a thread of its own, so that opening any of them is a
/// lookup.
///
/// A plain `std::thread`, not a task: it writes into a `Mutex` and needs nothing to poll it and
/// nothing to notice when it finishes, which is why it works whatever the window's event loop is
/// doing. `still_wanted` is asked between messages, so a selection that has moved on abandons
/// what it was warming rather than finishing it.
pub fn warm(
    store: std::sync::Arc<SqliteStore>,
    messages: Vec<Message>,
    policy: SanitizePolicy,
    still_wanted: impl Fn() -> bool + Send + 'static,
) {
    let _ = std::thread::Builder::new()
        .name("reader-warm".to_owned())
        .spawn(move || {
            for message in &messages {
                if !still_wanted() {
                    return;
                }
                if peek(message, policy).is_none() {
                    let _ = rendered(&store, message, policy);
                }
            }
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
        assert_eq!(cache.entries.len(), 1);
    }

    #[test]
    fn putting_a_key_twice_does_not_count_it_twice() {
        let mut cache = Cache::new(100);
        let k = key(1, RemoteImages::Blocked, 1);
        cache.put(k, html(60));
        cache.put(k, html(60));
        assert_eq!(cache.held, 60);
        assert_eq!(cache.entries.len(), 1);
    }
}
