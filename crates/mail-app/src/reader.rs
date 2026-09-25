//! What the reader needs from the store that is not already a column.
//!
//! One function so far, and it exists because the HTML part of a message is not stored
//! separately: it lives inside the raw bytes, which are the only copy that is exactly what the
//! server sent. `ui.rs` is deliberately thin, and this does I/O, so it is neither there nor in
//! `view.rs` — which is I/O-free on purpose.

use crate::view::{Reading, reading};
use mail_domain::Message;
use mail_mime::{Limits, SanitizePolicy};
use mail_store::SqliteStore;

/// Everything the reader needs for one message, resolved.
///
/// The block walk resolves `cid:` and remote images. It runs on sanitized markup, so it
/// judges the URLs the message contains. The frame, when there is one, is that same
/// policy's markup with no second embed pass: inline images live in the blocks.
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
    /// The block limits' revision. Its own number, not the sanitizer's: a cap or a mapping
    /// change alters the blocks without altering the markup, and one shared number would be
    /// bumped by whoever next edited the other.
    blocks: u32,
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
    // `Document::bytes` is counted once, when the document is built. Walking the tree on
    // every insert and every eviction would be O(blocks) against a 16 MiB budget.
    // The frame's markup is stored beside the blocks, so it counts too.
    match reading {
        Reading::NotFetched => 0,
        Reading::Blocks { document, html, .. } => {
            document.bytes() + html.as_ref().map(String::len).unwrap_or(0) + fetched(reading)
        }
        Reading::Layout { document, html, .. } => document.bytes() + html.len() + fetched(reading),
    }
}

/// The bytes of the frame's fetch list, stored beside its markup.
fn fetched(reading: &Reading) -> usize {
    reading.frame_fetches().iter().map(String::len).sum()
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
///
/// The shell calls this with [`SanitizePolicy::CURRENT`] while the open reader uses
/// [`crate::view::Shell::policy`]. That split is deliberate. Consent is per thread and is
/// revoked by [`crate::view::Shell::open`] and [`crate::view::Shell::select`], so the next
/// thread opens with remote images blocked — which is `CURRENT`. Warming whatever policy the
/// thread on screen happens to be using would fill the cache with network-fetching URLs for
/// conversations nobody has opened.
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
    render_at_limits(store, message, policy, Limits::version())
}

/// [`render`] as if the block limits were at revision `blocks`.
///
/// The document is still built by the code in this binary. The argument is only the cache
/// key, so a test can show that a new revision does not reuse an old entry.
#[doc(hidden)]
pub fn render_at_limits(
    store: &SqliteStore,
    message: &Message,
    policy: SanitizePolicy,
    blocks: u32,
) -> Reading {
    // Answered from the cache when it has been asked before — phase 8d. The key is the whole
    // input, so a hit is the same answer this function would compute.
    let key = match &message.body {
        mail_domain::Body::Present { raw, .. } => Some(Key {
            raw: *raw,
            remote_images: policy.remote_images,
            version: policy.version,
            blocks,
        }),
        // Nothing fetched yet: no bytes, no key, and the answer is a constant anyway.
        mail_domain::Body::Absent => None,
    };
    if let Some(key) = key
        && let Some(had) = cached(&key)
    {
        return had;
    }

    // Parsed once. The shell re-renders the open thread on every keystroke, and a second
    // parse of every message in it is the difference between a reader that opens and one
    // that stutters.
    let parsed = parse_body(store, message);
    let rendered = reading(&message.body, parsed.as_ref(), policy);
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
