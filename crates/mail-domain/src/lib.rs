//! Product vocabulary for the mail client. Pure data and pure functions.
//!
//! This crate has no I/O, no async, no clock, and no knowledge of SQL or of any wire
//! protocol's syntax. It does own [`RemoteRef`] and [`ProtoOp`] — "this message, over there"
//! and "do this remotely" are product vocabulary that `mail-store` must persist, and
//! `mail-store` cannot see `mail-proto`.
//!
//! See `plan.md` for the design and `CONVENTIONS.md` for the rules this code follows.

pub mod account;
pub mod content;
pub mod draft;
pub mod filter;
pub mod id;
pub mod ingest;
pub mod message;
pub mod op;
pub mod presets;
pub mod remote;
pub mod retry;
pub mod state;
pub mod threading;
pub mod view;

pub use account::{
    AccountCaps, AccountPlan, ArchiveMeans, AuthPlan, Condstore, ConnectionBudget, Credential,
    ExpungeMeans, FolderRoles, Identity, Incoming, LeaveOnServer, MoveExt, OAuthIssuer, Outgoing,
    SaslMech, SecretKey, SecretPurpose, ServerLabels, ServerThreads, Supported, Tls, Username,
    WatchMode,
};
pub use content::{Address, Attachment, Body, Inline, Label};
pub use draft::{Draft, PendingAttachment, ReplyScope, SendState};
pub use filter::{DateRange, Filter, MatchCtx, TextMatch};
pub use id::{
    AccountId, BlobId, ChangeId, DraftId, IdentityId, LabelId, MessageId, OutboxId, ThreadId,
    ViewId,
};
pub use ingest::{Fetched, Ingest};
pub use message::{Message, MessageKey, Thread, ThreadSummary};
pub use op::{Action, Applied, Change, Op, OpKind, Patch, RemoteIntent, Target};
pub use remote::{FetchSince, MailboxRef, ProtoOp, RemoteRef, SyncCursor, UidValidity};
pub use retry::{Retry, Retryable};
pub use state::{
    Attachments, IsDefault, LabelOrigin, MailboxRole, MailboxSet, Membership, Pin, ReadState,
    Snooze, Star, Threading,
};
pub use threading::{ThreadInput, normalize_id, thread};
pub use view::{Cursor, GroupKey, Page, PageReq, Property, Query, Sort, SortDir, View, ViewKind};
