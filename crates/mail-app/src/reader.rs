//! What the reader needs from the store that is not already a column.
//!
//! One function so far, and it exists because the HTML part of a message is not stored
//! separately: it lives inside the raw bytes, which are the only copy that is exactly what the
//! server sent. `ui.rs` is deliberately thin, and this does I/O, so it is neither there nor in
//! `view.rs` — which is I/O-free on purpose.

use crate::view::{Reading, reading};
use mail_domain::Message;
use mail_mime::SanitizePolicy;
use mail_store::SqliteStore;

/// Everything the reader needs for one message, resolved.
///
/// Sanitize first, then resolve `cid:` inside the result. That order is the point: the
/// sanitizer must judge the URLs the *message* contains, not a `data:` URI substituted for one
/// — and running it second would mean re-judging bytes this code produced rather than bytes the
/// sender did.
/// The most rendered output to keep, in bytes.
///
/// Sixteen mebibytes, and bounded by *bytes* rather than by a count because what is cached is
/// mostly inline images: one message with a photograph in it outweighs a hundred without.
const CACHE_BUDGET: usize = 16 * 1024 * 1024;

/// What a rendering is a function of.
///
/// The whole of the input, which is the point. `raw` is a `BlobId` and `BlobStore::put` is
/// content-addressed — the same bytes are the same blob — so a message's rendered form cannot
/// change while its key stays the same. There is nothing to invalidate, and no way for the cache
/// to hold an answer that has quietly stopped being true.
///
/// That is a property of the design rather than a lucky fact about this function. `render` reads
/// a blob and calls pure code on it; in a client where a message were a mutable object with a
/// body that could be edited in place, this cache would be a bug farm and the key would have to
/// be a version number somebody remembered to bump.
#[derive(PartialEq, Eq, Clone, Copy)]
struct Key {
    raw: mail_domain::BlobId,
    remote_images: mail_mime::RemoteImages,
    /// The sanitizer's revision. Its own documentation already called this "part of the render
    /// cache key", years before there was one: without it an upgrade would keep serving markup
    /// judged under the old rules.
    version: u32,
}

/// Rendered messages, newest use last.
///
/// A `Vec` rather than a map: the budget keeps it to a few dozen entries, a linear scan of that
/// is faster than hashing a key, and the order *is* the eviction policy.
struct Cache {
    entries: Vec<(Key, Reading)>,
    bytes: usize,
}

fn weight(reading: &Reading) -> usize {
    match reading {
        Reading::NotFetched => 0,
        Reading::Text(text) => text.len(),
        Reading::Html { html, .. } => html.len(),
    }
}

static CACHE: std::sync::Mutex<Cache> = std::sync::Mutex::new(Cache {
    entries: Vec::new(),
    bytes: 0,
});

/// The cached rendering for `key`, if there is one, moved to the back as the most recent.
fn cached(key: &Key) -> Option<Reading> {
    // A poisoned lock is not a reason to fail a render: the cache holds nothing that matters,
    // and the worst case is doing the work again.
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    let at = cache.entries.iter().position(|(had, _)| had == key)?;
    let entry = cache.entries.remove(at);
    let found = entry.1.clone();
    cache.entries.push(entry);
    Some(found)
}

fn remember(key: Key, reading: &Reading) {
    let cost = weight(reading);
    // Nothing to gain, and a single message larger than the whole budget would evict everything
    // else to hold only itself.
    if cost == 0 || cost > CACHE_BUDGET {
        return;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.entries.push((key, reading.clone()));
    cache.bytes += cost;
    while cache.bytes > CACHE_BUDGET && !cache.entries.is_empty() {
        let (_, dropped) = cache.entries.remove(0);
        cache.bytes = cache.bytes.saturating_sub(weight(&dropped));
    }
}

/// Render `messages` into the cache, off whatever thread the caller is on.
///
/// Phase 8e, and the reason it can exist at all while F140 stands: this writes into a `Mutex`,
/// not into a signal. It needs no render, no waker and no runtime — an ordinary thread fills the
/// cache and whatever draws next finds the answer already there. Everything else in phase 8 that
/// wanted to leave the render thread needed a way back onto it; this one does not.
///
/// What it buys: opening a conversation costs a lookup rather than a parse, a sanitize and an
/// inline-embed per message. Other clients do that work when you click. This does it before.
///
/// Returns how many were rendered, so a caller can say what it warmed and a test can tell the
/// difference between "did the work" and "found it already done".
pub fn prewarm(store: &SqliteStore, messages: &[Message], policy: SanitizePolicy) -> usize {
    let mut done = 0;
    for message in messages {
        // Stop at the budget rather than churning through it: warming more than the cache can
        // hold would evict the conversations the user is nearest to in order to hold the ones
        // they are furthest from.
        if fits(message) {
            let _ = render(store, message, policy);
            done += 1;
        }
    }
    done
}

/// Whether a message is worth warming: it has a body, and we are not already holding it.
fn fits(message: &Message) -> bool {
    matches!(message.body, mail_domain::Body::Present { .. })
}

/// How many renderings are being held. Integration tests count this process-wide cache.
#[doc(hidden)]
pub fn held() -> usize {
    CACHE
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .entries
        .len()
}

/// Forget everything cached. Integration tests share this process-wide cache.
#[doc(hidden)]
pub fn forget_everything() {
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.entries.clear();
    cache.bytes = 0;
}

pub fn render(store: &SqliteStore, message: &Message, policy: SanitizePolicy) -> Reading {
    // Answered from the cache when it has been asked before — phase 8d. The key is the whole
    // input, so a hit is the same answer this function would compute.
    let key = match &message.body {
        mail_domain::Body::Present { raw, .. } => Some(Key {
            raw: *raw,
            remote_images: policy.remote_images,
            version: policy.version,
        }),
        // Nothing fetched yet: no bytes, no key, and the answer is a constant anyway.
        mail_domain::Body::Absent => None,
    };
    if let Some(key) = key
        && let Some(had) = cached(&key)
    {
        return had;
    }

    // Parsed once. `html_of` and `inline_parts` each read the blob and run the whole MIME
    // parser, and `render` called both — so every message in an open thread was parsed twice on
    // every render, and the shell re-renders the open thread on every keystroke in the search
    // box. On a thread of large messages that is the difference between a reader that opens and
    // one that stutters.
    let Some(parsed) = parse_body(store, message) else {
        return reading(&message.body, None, policy);
    };
    let rendered = match reading(&message.body, parsed.html.as_deref(), policy) {
        Reading::Html {
            html,
            blocked_remote,
        } => Reading::Html {
            html: mail_mime::embed_inline(&html, &parsed.attachments, mail_mime::INLINE_BUDGET),
            blocked_remote,
        },
        other => other,
    };
    if let Some(key) = key {
        remember(key, &rendered);
    }
    rendered
}

/// The message's stored bytes, parsed.
///
/// `None` for a body that has not arrived, a blob that is missing, or bytes that do not parse —
/// the reader's answer to all three is the same, and it is the text part.
fn parse_body(store: &SqliteStore, message: &Message) -> Option<mail_mime::Parsed> {
    let raw = message.body.raw()?;
    let bytes = store.blobs().get(&store.connection(), raw).ok()?;
    mail_mime::parse(&bytes).ok()
}
