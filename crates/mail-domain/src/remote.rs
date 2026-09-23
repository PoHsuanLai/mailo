//! How local objects relate to what is on a server, and what we ask a server to do.
//!
//! These types live in the domain rather than in a protocol crate because `mail-store`
//! persists them — `remote_map`, `sync_state` and `outbox` — and `mail-store` does not depend
//! on `mail-proto`. They are protocol-neutral vocabulary; wire *syntax* stays in `mail-proto`.

use crate::id::{AccountId, BlobId, DraftId};
use crate::state::MailboxRole;
use serde::{Deserialize, Serialize};

/// One server-side mailbox. `"INBOX"` for POP3, which has only one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MailboxRef {
    pub account: AccountId,
    /// The IMAP folder path exactly as the server spells it, e.g. `"[Gmail]/All Mail"`.
    pub path: String,
}

/// Where a message sits on a server.
///
/// **This is many-to-one with [`crate::MessageId`], not a bijection.** On Gmail the same
/// message exists in `INBOX` and `[Gmail]/All Mail` under *different* UIDs, and again in
/// `[Gmail]/Sent` if you sent it. Identity comes from [`crate::MessageKey`]; a `RemoteRef` is
/// only an address. Treating it as an identity duplicates every message on the first sync.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum RemoteRef {
    Imap {
        mailbox: String,
        /// Scopes `uid`. When the server reports a different value, every `uid` for this
        /// mailbox is meaningless and the mapping must be rebuilt.
        uidvalidity: u32,
        uid: u32,
    },
    Pop {
        uidl: String,
    },
}

/// How far a mailbox has been synced. Per mailbox, not per account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum SyncCursor {
    Imap {
        uidvalidity: u32,
        uidnext: u32,
        /// `HIGHESTMODSEQ`, when the server offers `CONDSTORE`. Enables incremental flag
        /// sync instead of refetching every flag on every poll.
        modseq: Option<u64>,
    },
    /// POP3 keeps no cursor: every poll lists `UIDL` in full and diffs against `remote_map`.
    Pop,
}

/// Whether the server's `UIDVALIDITY` still matches what we stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UidValidity {
    Same,
    /// Every `remote_map` row for this mailbox must be dropped and the mailbox refetched.
    Reset,
}

impl UidValidity {
    /// Whether the mailbox the server just described is the one we hold rows for.
    ///
    /// `UIDVALIDITY` is IMAP's answer to "are these UIDs still the same UIDs". A server that
    /// recreates a mailbox — restored from backup, migrated, or a folder deleted and remade with
    /// the same name — must change it, and every UID we stored then addresses a different
    /// message or none at all. Continuing to use them is not a stale cache; it is marking the
    /// wrong mail read and fetching bodies onto the wrong headers.
    ///
    /// Deliberately conservative in three places, because [`UidValidity::Reset`] throws away
    /// every `remote_map` row for the mailbox and makes the next sync refetch it whole:
    ///
    /// - No stored cursor is a first sync. There is nothing to invalidate.
    /// - A zero on either side means the server did not say, and "did not say" is not "changed".
    /// - A POP cursor on either side has no `UIDVALIDITY` to compare; POP3 identity is the UIDL
    ///   and is handled by its own diff.
    pub fn between(stored: Option<&SyncCursor>, fresh: &SyncCursor) -> UidValidity {
        let (
            Some(SyncCursor::Imap {
                uidvalidity: was, ..
            }),
            SyncCursor::Imap {
                uidvalidity: now, ..
            },
        ) = (stored, fresh)
        else {
            return UidValidity::Same;
        };
        if *was != 0 && *now != 0 && was != now {
            UidValidity::Reset
        } else {
            UidValidity::Same
        }
    }
}

/// A mailbox state the server can resynchronise from: `SELECT … (QRESYNC (uidvalidity modseq))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resync {
    pub uidvalidity: u32,
    pub modseq: u64,
}

/// Where to resume a fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum FetchSince {
    Beginning,
    After { cursor: SyncCursor },
}

/// A unit of remote work. Queued in the outbox, retried with backoff, drained serially per
/// account so that two operations on one thread have a defined result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum ProtoOp {
    /// Refresh [`crate::AccountCaps`] from `CAPABILITY` / `CAPA`.
    FetchCaps,
    /// `LIST` the server's folders, to rebuild [`crate::FolderRoles`].
    ListFolders,
    FetchEnvelopes {
        mailbox: MailboxRef,
        since: FetchSince,
    },
    /// Headers only, without marking the message read.
    ///
    /// POP3 `TOP n 0` and IMAP envelope fetches both do this, for the same reason: retrieving
    /// whole messages to populate a list view is slow, and on POP3 it is also destructive —
    /// `RETR` sets the seen flag on Dovecot and `TOP` does not, so a first sync over a large
    /// maildrop would mark the user's entire mailbox read in their webmail.
    FetchHeaders {
        /// A batch, not one message.
        ///
        /// A connection is authenticated once and then used for many commands. Asking for one
        /// message per operation means either a new connection each time — 2372 of them on the
        /// maildrop we measured, which a server will rate-limit — or a session waiting forever
        /// for a greeting the connection already consumed. An end-to-end test found the second.
        remotes: Vec<RemoteRef>,
    },
    FetchBody {
        remotes: Vec<RemoteRef>,
    },
    SetFlags {
        remotes: Vec<RemoteRef>,
        read: Option<crate::state::ReadState>,
        star: Option<crate::state::Star>,
    },
    SetMailbox {
        remotes: Vec<RemoteRef>,
        role: MailboxRole,
    },
    SetLabels {
        remotes: Vec<RemoteRef>,
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// Add a keyword to messages: `UID STORE +FLAGS (…)` on IMAP. A server that cannot keep
    /// keywords, and POP3, which has no flags at all, treat it as done.
    AddKeyword {
        remotes: Vec<RemoteRef>,
        keyword: crate::receipt::Keyword,
    },
    /// Upload a message we composed, e.g. a draft to the Drafts folder.
    Append {
        mailbox: MailboxRef,
        raw: BlobId,
        role: MailboxRole,
    },
    /// Hand one composed message to the submission server.
    ///
    /// Everything here is frozen at the moment the user pressed send, which is why the bytes
    /// are a [`BlobId`] rather than a draft to re-render: a draft edited while the outbox was
    /// backed off would otherwise send the edit, not what was sent.
    Submit {
        draft: DraftId,
        /// The exact bytes to transmit.
        raw: BlobId,
        /// `MAIL FROM` — the return path for bounces.
        mail_from: String,
        /// `RCPT TO`: `To`, `Cc` **and** `Bcc`.
        ///
        /// Carried here rather than re-derived from `raw`, and that is the whole point. The
        /// transmitted headers deliberately omit `Bcc` (FINDINGS F37), so a recipient list
        /// parsed back out of the bytes would silently drop every blind recipient — on the
        /// retry, not on the first attempt, which is the worst way to find out.
        rcpt_to: Vec<String>,
    },
    /// Flag changes since a known point, without refetching anything else.
    ///
    /// The second of the three things a sync pass must do. Gmail's IDLE reports **new mail
    /// only** — a message read or starred on another device never arrives through it — so
    /// without a timed sweep the client never learns that anything changed elsewhere. With
    /// `CONDSTORE` this is one `CHANGEDSINCE` fetch and costs almost nothing; without it, or
    /// where `HIGHESTMODSEQ` is observed not to advance, it degrades to a full flag fetch on a
    /// longer interval.
    FetchFlags {
        mailbox: MailboxRef,
        /// `None` means "everything": no modseq, or one we no longer trust.
        since_modseq: Option<u64>,
    },
    /// What was expunged from this mailbox elsewhere.
    ///
    /// The third thing a sync pass must do. Without `QRESYNC` — Gmail does not offer it — the
    /// only way is to list every UID the server holds, and RFC 7162 says outright that a
    /// CONDSTORE-only client "still has to issue a UID FETCH or a UID SEARCH"; the caller diffs
    /// that listing against `remote_map`. With `since`, and a server that has `QRESYNC`, the
    /// server names what vanished itself and there is no listing to diff.
    ListRemote {
        mailbox: MailboxRef,
        /// Where the caller's knowledge of this mailbox stands, for `QRESYNC`. `None` asks for
        /// the full listing, which is always correct and is what every server without it gets.
        #[serde(default)]
        since: Option<Resync>,
    },
    Expunge {
        remotes: Vec<RemoteRef>,
    },
    /// The MIME structure of each message, and none of its content: IMAP `BODYSTRUCTURE`.
    ///
    /// Asked only about messages large enough for the answer to pay for itself. F113 took this
    /// out of the envelope walk because nothing read it and it cost eleven times the response;
    /// here the code that reads it exists, and it decides which megabytes not to download.
    FetchStructure {
        remotes: Vec<RemoteRef>,
    },
    /// Named sections of one message: `HEADER`, `2.MIME`, `2.1`. IMAP only.
    FetchSections {
        remote: RemoteRef,
        sections: Vec<String>,
    },
    Watch {
        mailbox: MailboxRef,
    },
    /// Create, rename, delete or follow a mailbox. IMAP only.
    Folder(crate::folder::FolderWork),
}

#[cfg(test)]
mod uidvalidity_tests {
    use super::*;

    fn imap(uidvalidity: u32) -> SyncCursor {
        SyncCursor::Imap {
            uidvalidity,
            uidnext: 10,
            modseq: None,
        }
    }

    #[test]
    fn a_changed_uidvalidity_invalidates_the_mailbox() {
        // The whole point: every stored UID now addresses a different message, or none.
        assert_eq!(
            UidValidity::between(Some(&imap(42)), &imap(43)),
            UidValidity::Reset
        );
    }

    #[test]
    fn an_unchanged_uidvalidity_keeps_everything() {
        assert_eq!(
            UidValidity::between(Some(&imap(42)), &imap(42)),
            UidValidity::Same
        );
    }

    #[test]
    fn a_first_sync_has_nothing_to_invalidate() {
        assert_eq!(UidValidity::between(None, &imap(42)), UidValidity::Same);
    }

    #[test]
    fn silence_is_not_a_change() {
        // Zero is what this code records when the server said nothing. Treating that as a reset
        // would refetch the entire mailbox every time a server omitted the response code.
        assert_eq!(
            UidValidity::between(Some(&imap(0)), &imap(42)),
            UidValidity::Same
        );
        assert_eq!(
            UidValidity::between(Some(&imap(42)), &imap(0)),
            UidValidity::Same
        );
    }

    #[test]
    fn pop_has_no_uidvalidity_to_compare() {
        assert_eq!(
            UidValidity::between(Some(&SyncCursor::Pop), &imap(42)),
            UidValidity::Same
        );
        assert_eq!(
            UidValidity::between(Some(&imap(42)), &SyncCursor::Pop),
            UidValidity::Same
        );
    }
}
