//! The reader's consent to remote images, as the network sees it.
//!
//! The reader decides (`Shell::show_remote_images`: per thread, never persisted, revoked by
//! opening, selecting or closing). This is where that decision is held for whoever answers the
//! Original frame's requests, which run outside any component: on Blitz, `net.rs`'s `AppNet`.
//!
//! What a grant admits is narrow on purpose:
//! - only `http` and `https`, and only a URL the sanitizer kept in a fetch attribute of one of
//!   the consented thread's messages ([`crate::view::Reading::frame_fetches`]);
//! - per message: a frame may fetch only what its own message's markup asks for. Each Original
//!   frame carries its message's id as its `data-frame-tag`, and the network reads the tag of the
//!   frame that asked (`net.rs`), so one message's frame cannot fetch another message's image.
//! - while it stands: a new grant, or none, makes every earlier admission stale, and a fetch
//!   that lands after that is dropped rather than shown.

use mail_domain::{MessageId, ThreadId};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The reader's consent, shared between the reader that grants it and the network that reads it.
/// A root context on the window; cloning shares it.
#[derive(Clone, Default)]
pub struct Consent(Arc<Mutex<Grant>>);

impl std::fmt::Debug for Consent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Consent")
            .field("granted", &self.granted())
            .finish()
    }
}

/// One reader's claim on the consent: the reader that holds the grant is the one whose closing
/// takes it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Holder(u64);

/// A request admitted under one grant. It stands until the grant changes.
///
/// Only the native network reads the consent (the webview's frames are held by their markup), so
/// the reading side is unused in a webview build.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket(u64);

#[derive(Default)]
struct Grant {
    /// Moved by every change of grant; a [`Ticket`] carries the value it was admitted under.
    generation: u64,
    /// Handed out by [`Consent::holder`].
    holders: u64,
    /// Who granted what stands now, and for which thread.
    by: Option<(Holder, ThreadId)>,
    /// Each consented message and the URLs its frame may fetch, normalised.
    messages: Vec<(MessageId, Vec<String>)>,
}

impl Consent {
    /// Nothing consented to.
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    pub fn new() -> Self {
        Consent::default()
    }

    fn grant(&self) -> MutexGuard<'_, Grant> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A name for one reader, to hold and release the grant with.
    pub(crate) fn holder(&self) -> Holder {
        let mut grant = self.grant();
        grant.holders = grant.holders.wrapping_add(1);
        Holder(grant.holders)
    }

    /// `holder` shows `thread`: with its images allowed (`Some`, each message and what its frame
    /// fetches), or not (`None`).
    ///
    /// Called on every render of the reader, before the render reaches the document, so a frame
    /// that reloads with the consented markup finds its grant already there. The same grant again
    /// changes nothing, and admissions under it stay good.
    pub(crate) fn hold(
        &self,
        holder: Holder,
        thread: ThreadId,
        allowed: Option<Vec<(MessageId, Vec<String>)>>,
    ) {
        let mut grant = self.grant();
        match allowed {
            None => {
                if grant.by.is_some() {
                    grant.revoke();
                }
            }
            Some(lists) => {
                let messages: Vec<(MessageId, Vec<String>)> = lists
                    .into_iter()
                    .map(|(id, urls)| (id, urls.iter().filter_map(|url| normal(url)).collect()))
                    .collect();
                if grant.by == Some((holder, thread)) && grant.messages == messages {
                    return;
                }
                grant.revoke();
                grant.by = Some((holder, thread));
                grant.messages = messages;
            }
        }
    }

    /// `holder` is gone (the reader closed): what it granted is taken back. A grant another
    /// reader holds is left alone.
    pub(crate) fn release(&self, holder: Holder) {
        let mut grant = self.grant();
        if grant.by.is_some_and(|(by, _)| by == holder) {
            grant.revoke();
        }
    }

    /// Whether the frame showing `message` may fetch `url` now.
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    pub(crate) fn admits(&self, message: MessageId, url: &str) -> Option<Ticket> {
        let url = normal(url)?;
        let grant = self.grant();
        grant.by?;
        grant
            .messages
            .iter()
            .any(|(id, urls)| *id == message && urls.contains(&url))
            .then_some(Ticket(grant.generation))
    }

    /// Whether what `ticket` admitted may still be shown.
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    pub(crate) fn stands(&self, ticket: Ticket) -> bool {
        let grant = self.grant();
        grant.by.is_some() && grant.generation == ticket.0
    }

    /// Whether any thread's images are allowed now.
    pub fn granted(&self) -> bool {
        self.grant().by.is_some()
    }
}

impl Grant {
    fn revoke(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.by = None;
        self.messages.clear();
    }
}

/// `url` as the renderer asks for it, when it is a web address; nothing else is ever admitted.
///
/// The sanitizer keeps a URL as the sender wrote it; a renderer asks for it parsed. Parsing both
/// the same way makes them comparable (`HTTPS://Cdn.Test/a.png` is `https://cdn.test/a.png`).
fn normal(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.into()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod consent_tests;
