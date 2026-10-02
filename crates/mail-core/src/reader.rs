//! What the reader needs from the store that is not already a column.
//!
//! One function so far, and it exists because the HTML part of a message is not stored
//! separately: it lives inside the raw bytes, which are the only copy that is exactly what the
//! server sent. `ui.rs` is deliberately thin, and this does I/O, so it is neither there nor in
//! `view.rs` — which is I/O-free on purpose.

use mail_domain::{Body, Message};
use mail_mime::{Block, Document, Flowed, ImgSrc, Limits, RemoteImages, SanitizePolicy, Shape};
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

/// What the reader should display for one message.
///
/// Plain text is blocks, through [`mail_mime::from_text`]. HTML is blocks too.
/// [`Reading::Layout`] keeps the sanitized markup beside them because the body
/// scored [`Shape::Layout`]: the blocks open, and the markup is the Original frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// Headers only so far. Normal mid-sync, and not an empty message.
    NotFetched,
    /// Blocks. `html` is the sanitized markup when the body had an HTML part that
    /// did not score [`Shape::Layout`]. Part D's sketch had no such field; it is here
    /// because the frame is the Original view for every HTML body, a letter or a
    /// receipt included, and the reader's frame tests hold an iframe for both.
    /// Plain text leaves it `None`: there is nothing original to show.
    Blocks {
        document: Document,
        /// Whether a remote image was held back. The reader offers to load them only
        /// when there is something to load.
        blocked_remote: bool,
        html: Option<String>,
        /// What the frame's markup fetches: [`Reading::frame_fetches`].
        fetches: Vec<String>,
    },
    /// Blocks and the sanitized markup, because [`Document::shape`] is [`Shape::Layout`].
    ///
    /// Both, not either: the blocks are what opens, and the markup is the Original frame.
    Layout {
        document: Document,
        html: String,
        blocked_remote: bool,
        /// What the frame's markup fetches: [`Reading::frame_fetches`].
        fetches: Vec<String>,
    },
}

impl Reading {
    /// The block document, when the body has been fetched.
    pub fn document(&self) -> Option<&Document> {
        match self {
            Reading::NotFetched => None,
            Reading::Blocks { document, .. } | Reading::Layout { document, .. } => Some(document),
        }
    }

    /// Sanitized markup for the sandboxed frame, when this body has one.
    pub fn frame_html(&self) -> Option<&str> {
        match self {
            Reading::Layout { html, .. } => Some(html),
            Reading::Blocks { html, .. } => html.as_deref(),
            Reading::NotFetched => None,
        }
    }

    /// Every remote URL the frame's markup fetches as it is shown, as the sanitizer kept them
    /// ([`mail_mime::SafeHtml::remote_fetches`]): empty unless the policy allowed remote images.
    ///
    /// The allowlist the Original frame's network is held to on Blitz: the frame may ask for
    /// these and nothing else, and only while the reader's consent stands (`ui/original`).
    pub fn frame_fetches(&self) -> &[String] {
        match self {
            Reading::Blocks { fetches, .. } | Reading::Layout { fetches, .. } => fetches,
            Reading::NotFetched => &[],
        }
    }

    /// Whether a remote image was held back.
    pub fn blocked_remote(&self) -> bool {
        match self {
            Reading::Blocks { blocked_remote, .. } | Reading::Layout { blocked_remote, .. } => {
                *blocked_remote
            }
            Reading::NotFetched => false,
        }
    }
}

/// Decide what to show for a message body.
///
/// `parsed` is the whole message: blocks need its parts for `cid:` and the
/// text part's flowed parameters. HTML is sanitized here rather than at ingest,
/// so a policy change takes effect immediately.
pub fn reading(body: &Body, parsed: Option<&mail_mime::Parsed>, policy: SanitizePolicy) -> Reading {
    match body {
        Body::Absent => Reading::NotFetched,
        Body::Present { text, .. } => {
            let Some(parsed) = parsed else {
                return plain(text.as_deref().unwrap_or(""), Flowed::Fixed);
            };
            if let Some(html) = parsed.html.as_deref() {
                html_reading(html, parsed, policy)
            } else {
                let source = parsed.text.as_deref().or(text.as_deref()).unwrap_or("");
                plain(source, parsed.flowed)
            }
        }
    }
}

fn plain(text: &str, flowed: Flowed) -> Reading {
    Reading::Blocks {
        document: mail_mime::from_text(text, flowed),
        blocked_remote: false,
        html: None,
        fetches: Vec::new(),
    }
}

/// Blocks from sanitized HTML. Images resolve in that walk.
///
/// Remote images are a second pass only when the policy would strip their URLs
/// and the body actually has some. The block parser names a blocked image by
/// its host, and a host cannot be read from markup whose `src` is already gone.
/// The frame always gets the policy's own markup, so a blocked URL is not in
/// the document the Original tab holds.
fn html_reading(html: &str, parsed: &mail_mime::Parsed, policy: SanitizePolicy) -> Reading {
    let keep_urls = policy.remote_images == RemoteImages::Blocked && has_remote_url(html);
    let for_blocks = if keep_urls {
        mail_mime::sanitize(
            html,
            SanitizePolicy {
                remote_images: RemoteImages::Allowed,
                version: policy.version,
            },
        )
    } else {
        mail_mime::sanitize(html, policy)
    };
    let document = mail_mime::from_html(&for_blocks, &parsed.attachments, policy.remote_images);
    let frame = if keep_urls {
        mail_mime::sanitize(html, policy)
    } else {
        for_blocks
    };
    let blocked_remote = frame.blocked_remote() > 0 || image_blocked(&document.blocks);
    // The message's own inline images, as `data:` URIs (F42): the frame is a document of its
    // own on either renderer, where a `cid:` names nothing it can reach. After sanitizing, so
    // the sanitizer judged the `cid:` the sender wrote and never a URI made here.
    let html = mail_mime::embed_inline(
        frame.as_str(),
        &parsed.attachments,
        mail_mime::INLINE_BUDGET,
    );
    let fetches = frame.remote_fetches().to_vec();
    if document.shape == Shape::Layout {
        Reading::Layout {
            document,
            html,
            blocked_remote,
            fetches,
        }
    } else {
        Reading::Blocks {
            document,
            blocked_remote,
            html: Some(html),
            fetches,
        }
    }
}

/// Whether `html` mentions an `http` or `https` URL.
///
/// A performance choice, not a security boundary: a miss costs a host name on
/// the placeholder, and a hit costs a second sanitize. The policy still decides
/// what is fetched.
fn has_remote_url(html: &str) -> bool {
    // `match_indices` on the separator, then the scheme behind it: the search is std's
    // substring finder, where a byte-window scan cost 75 ms on a 2 MB body in a debug build.
    let bytes = html.as_bytes();
    html.match_indices("://").any(|(at, _)| {
        let ends = |scheme: &[u8]| {
            at >= scheme.len() && bytes[at - scheme.len()..at].eq_ignore_ascii_case(scheme)
        };
        ends(b"http") || ends(b"https")
    })
}

fn image_blocked(blocks: &[Block]) -> bool {
    let mut stack: Vec<&[Block]> = vec![blocks];
    while let Some(level) = stack.pop() {
        for block in level {
            match block {
                Block::Image {
                    src: ImgSrc::Blocked { .. },
                    ..
                } => return true,
                Block::Quote { blocks, .. } | Block::Signature(blocks) => stack.push(blocks),
                Block::List { items, .. } => {
                    for item in items {
                        stack.push(item);
                    }
                }
                _ => {}
            }
        }
    }
    false
}

#[cfg(test)]
mod reading_tests {
    use super::*;
    use mail_domain::BlobId;

    #[test]
    fn a_remote_url_is_noticed_whatever_its_case() {
        let cases: &[(&str, bool)] = &[
            ("<img src=\"https://a.test/x.png\">", true),
            ("<img src=\"HTTP://a.test/x.png\">", true),
            ("<a href=\"hTtPs://a.test\">", true),
            ("<img src=\"cid:pic@example\">", false),
            ("<p>see ftp://a.test</p>", false),
            ("://a.test at the very start", false),
        ];
        for (html, want) in cases {
            assert_eq!(has_remote_url(html), *want, "{html}");
        }
    }

    fn message_of(body: &str) -> mail_mime::Parsed {
        mail_mime::parse(body.as_bytes()).unwrap()
    }

    #[test]
    fn an_unfetched_body_is_not_an_empty_message() {
        assert_eq!(
            reading(&Body::Absent, None, SanitizePolicy::CURRENT),
            Reading::NotFetched
        );
    }

    #[test]
    fn html_is_sanitized_at_render_and_the_policy_is_honoured() {
        let body = Body::Present {
            text: Some("fallback".into()),
            raw: BlobId::generate(),
        };
        let hostile = "From: a@b.test\r\nSubject: s\r\nMIME-Version: 1.0\r\n\
             Content-Type: text/html; charset=utf-8\r\n\r\n\
             <p>hi</p><script>alert(1)</script><img src=\"https://tracker.test/p.gif\">";
        let parsed = message_of(hostile);

        let blocked = reading(&body, Some(&parsed), SanitizePolicy::CURRENT);
        let rendered = blocked.frame_html().expect("an html body has a frame");
        assert!(rendered.contains("<p>hi</p>"), "{rendered}");
        assert!(!rendered.contains("alert"), "script survived: {rendered}");
        assert!(
            !rendered.contains("tracker.test"),
            "a remote image survived blocking: {rendered}"
        );
        assert!(
            blocked.blocked_remote(),
            "the tracking image was not recorded as blocked"
        );

        let allowed = reading(
            &body,
            Some(&parsed),
            SanitizePolicy {
                remote_images: RemoteImages::Allowed,
                version: SanitizePolicy::CURRENT.version,
            },
        );
        let rendered = allowed.frame_html().expect("an html body has a frame");
        assert!(rendered.contains("tracker.test"), "opting in must work");
        assert!(!rendered.contains("alert"), "images are not scripts");
        assert!(!allowed.blocked_remote());
        // What the frame may fetch is what the sanitizer kept, and only once allowed.
        assert!(blocked.frame_fetches().is_empty(), "{blocked:?}");
        assert_eq!(
            allowed.frame_fetches(),
            ["https://tracker.test/p.gif".to_owned()]
        );
    }

    #[test]
    fn plain_text_is_used_when_there_is_no_html() {
        let body = Body::Present {
            text: Some("just text".into()),
            raw: BlobId::generate(),
        };
        let parsed = message_of("From: a@b.test\r\nSubject: s\r\n\r\njust text\r\n");
        let reading = reading(&body, Some(&parsed), SanitizePolicy::CURRENT);
        let Reading::Blocks {
            document,
            html: None,
            blocked_remote: false,
            ..
        } = reading
        else {
            panic!("plain text should be blocks without a frame: {reading:?}");
        };
        let text = document
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Paragraph { spans, .. } => Some(spans),
                _ => None,
            })
            .expect("a paragraph");
        assert!(
            matches!(text.as_slice(), [mail_mime::Span::Text(got)] if got == "just text"),
            "{text:?}"
        );
    }
}
