//! How large a message is, against the largest its account's server takes: said in the composer
//! before Send, rather than learnt from a refusal after it.
//!
//! The limit is what the sending server said, where it said one: SMTP's `SIZE` in the `EHLO` of
//! the last submission, or a JMAP session's upload limits, both kept beside the account's
//! capabilities as the runtime hears them. Microsoft Graph says nothing, and takes what its
//! upload sessions take. Anything else is measured against [`USUAL_LIMIT`].
//!
//! The two kinds of limit count different things. SMTP and JMAP count the message as it goes,
//! attachments in base64; Graph and the usual limit count what the person attached. A [`Weight`]
//! holds both, and [`Weight::against`] reads the one a limit means.

use mail_domain::{Draft, Outgoing};
use mail_store::SqliteStore;
use porter_core::AccountId;

use super::pictures::{self, PictureSize};
use crate::error::Logged;

/// The limit a message is measured against when its server has said none: the 25 MB of
/// attachments Gmail, Outlook.com and most others refuse past.
pub const USUAL_LIMIT: u64 = 25 * 1024 * 1024;

/// What a message's headers and parts' own headers come to, beside what they hold: a
/// generous guess, since a few kilobytes either way does not move a limit of megabytes.
const HEADERS: u64 = 4 * 1024;
const PART_HEADERS: u64 = 512;

/// What a limit counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    /// The message as it goes: attachments in base64. SMTP's `SIZE` and JMAP's uploads.
    Wire,
    /// What was written and attached, as it was: Graph's limit, and the usual one.
    Content,
}

/// Where a limit came from, which is how the composer names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Said {
    /// The sending server said it.
    Server,
    /// Microsoft Graph's upload sessions take no more.
    Graph,
    /// Nothing said it: [`USUAL_LIMIT`].
    Usual,
}

/// The largest message an account sends, and what it counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendLimit {
    pub octets: u64,
    pub measure: Measure,
    pub said: Said,
}

impl SendLimit {
    /// The limit nothing said.
    pub const USUAL: SendLimit = SendLimit {
        octets: USUAL_LIMIT,
        measure: Measure::Content,
        said: Said::Usual,
    };
}

/// The largest message `account` sends: what its server said, where it said one.
pub fn send_limit(store: &SqliteStore, account: &AccountId) -> SendLimit {
    let plan = store
        .account(account.clone())
        .or_log("the sending account could not be read")
        .flatten()
        // A plan that does not read says nothing of how it sends: the stored limit still does.
        .and_then(|stored| stored.plan.ok());
    match plan.map(|plan| plan.outgoing) {
        Some(Outgoing::Graph) => SendLimit {
            octets: mail_runtime::graph::MAX_MESSAGE,
            measure: Measure::Content,
            said: Said::Graph,
        },
        Some(Outgoing::Nowhere) => SendLimit::USUAL,
        // A plan that does not read still sends through the server that said what it takes.
        Some(Outgoing::Smtp { .. } | Outgoing::Jmap) | None => store
            .send_limit(account.clone())
            .or_log("the sending server's size limit could not be read")
            .flatten()
            .map_or(SendLimit::USUAL, |octets| SendLimit {
                octets,
                measure: Measure::Wire,
                said: Said::Server,
            }),
    }
}

/// How large a message is at each [`PictureSize`], both ways a limit counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Weight {
    /// As it goes, smallest picture size first.
    wire: [u64; 4],
    /// As written and attached, smallest picture size first.
    content: [u64; 4],
    /// How many of its files are pictures a smaller size could change.
    pictures: usize,
}

impl Weight {
    /// A message of `body` octets of text carrying `files`: each its size at every picture size,
    /// smallest first, and whether it is a picture.
    pub fn of(body: u64, files: &[([u64; 4], Picture)]) -> Weight {
        let mut wire = [HEADERS + mail_mime::base64_len(body); 4];
        let mut content = [body; 4];
        for (sizes, _) in files {
            for at in 0..4 {
                wire[at] += PART_HEADERS + mail_mime::base64_len(sizes[at]);
                content[at] += sizes[at];
            }
        }
        let pictures = files
            .iter()
            .filter(|(_, picture)| *picture == Picture::Yes)
            .count();
        Weight {
            wire,
            content,
            pictures,
        }
    }

    /// How large the message is at `size`, counted as `limit` counts.
    pub fn against(&self, size: PictureSize, limit: SendLimit) -> u64 {
        let at = PictureSize::ALL
            .iter()
            .position(|each| *each == size)
            .unwrap_or(3);
        match limit.measure {
            Measure::Wire => self.wire[at],
            Measure::Content => self.content[at],
        }
    }

    /// How many of the message's files are pictures Picture Size changes.
    pub fn pictures(&self) -> usize {
        self.pictures
    }
}

/// Whether a file is a picture a smaller size could change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picture {
    Yes,
    No,
}

/// Whether a message goes within its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Fits,
    /// Over it: the message's size and the limit, both as the limit counts, and the largest
    /// picture size smaller than the one chosen that would fit, when one would.
    Over {
        size: u64,
        limit: SendLimit,
        smaller: Option<PictureSize>,
    },
}

/// Whether a message of `weight`, its pictures sent at `chosen`, goes within `limit`.
pub fn fit(weight: &Weight, chosen: PictureSize, limit: SendLimit) -> Fit {
    let size = weight.against(chosen, limit);
    if size <= limit.octets {
        return Fit::Fits;
    }
    let smaller = match weight.pictures {
        0 => None,
        _ => PictureSize::ALL
            .into_iter()
            .rev()
            .filter(|each| *each < chosen)
            .find(|each| weight.against(*each, limit) <= limit.octets),
    };
    Fit::Over {
        size,
        limit,
        smaller,
    }
}

/// How large `draft` is, its stored files read and its pictures measured at every size. Reads
/// and decodes every picture, so it is for a thread that does not draw.
///
/// A file the store no longer has counts as nothing: it cannot be sent either, and Send says so.
pub fn weigh(store: &SqliteStore, draft: &Draft) -> Weight {
    let body = draft.text.len() + draft.html.as_ref().map_or(0, String::len);
    let files: Vec<([u64; 4], Picture)> = draft
        .attachments
        .iter()
        .filter_map(|attachment| {
            store
                .blobs()
                .get(attachment.blob)
                .or_log("an attachment could not be read to weigh it")
        })
        .map(|bytes| {
            let picture = if pictures::resizable(&bytes) {
                Picture::Yes
            } else {
                Picture::No
            };
            (pictures::sizes(&bytes), picture)
        })
        .collect();
    Weight::of(body as u64, &files)
}

#[cfg(test)]
#[path = "weight_tests.rs"]
mod tests;
