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
pub mod folder;
pub mod id;
pub mod ingest;
pub mod invite;
pub mod message;
pub mod op;
pub mod parts;
pub mod presets;
pub mod receipt;
pub mod remote;
pub mod retry;
pub mod rule;
pub mod state;
pub mod template;
pub mod threading;
pub mod view;

pub use account::{
    AccountCaps, AccountPlan, ArchiveMeans, AuthPlan, Condstore, ConnectionBudget, Credential,
    ExpungeMeans, FolderRoles, Identity, Incoming, LeaveOnServer, MoveExt, OAuthIssuer, Outgoing,
    SaslMech, SecretKey, SecretPurpose, ServerLabels, ServerThreads, Supported, Tls, Username,
    WatchMode,
};
pub use content::{Address, Attachment, Body, Inline, Label, PartContent};
pub use draft::{Draft, PendingAttachment, ReplyScope, SendState};
pub use filter::{DateRange, Filed, Filter, Leaving, MatchCtx, Placed, TextMatch};
pub use folder::{
    Folder, FolderContents, FolderCtx, FolderError, FolderWork, Holds, NonEmpty, SpecialUse,
    Subscription,
};
pub use id::{
    AccountId, BlobId, ChangeId, DraftId, IdentityId, LabelId, MessageId, OutboxId, RuleId,
    TemplateId, ThreadId, ViewId,
};
pub use ingest::{Fetched, Import, Ingest, Kept};
pub use invite::{Attendance, InviteAnswer};
pub use message::{Message, MessageKey, Thread, ThreadSummary};
pub use op::{Action, Applied, Change, Op, OpKind, Patch, RemoteIntent, Target};
pub use parts::PartTree;
pub use receipt::{Keyword, ReceiptAnswer, ReceiptRequest};
pub use remote::{
    FetchSince, MailboxRef, ProtoOp, RemoteRef, Resync, SyncCursor, SystemFlag, UidValidity,
};
pub use retry::{Retry, Retryable};
pub use rule::{AfterMatch, Rule, RuleAction, RuleState, Vacation};
pub use state::{
    Attachments, IsDefault, LabelOrigin, MailboxRole, MailboxSet, Membership, Pin, ReadState,
    Snooze, Star, Threading,
};
pub use template::Template;
pub use threading::{ThreadInput, normalize_id, thread};
pub use view::{Cursor, GroupKey, Page, PageReq, Property, Query, Sort, SortDir, View, ViewKind};
