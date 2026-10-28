//! What an account's incoming protocol can do, and where its server is.
//!
//! [`Incoming`] is the stored name of a protocol and stays an enum. The questions callers ask of
//! it that have a plain answer, whether it has a server, can search, can say how much storage is
//! used, are answered here once, in [`Incoming::caps`], so no caller lists protocols to ask.

use crate::account::{Incoming, Tls};

/// What an incoming protocol can do. A plain table: one row per protocol, built by
/// [`Incoming::caps`].
///
/// What the protocol *does*, signing in and fetching and searching, is the engines' business;
/// this holds only the facts about it a caller reads to decide whether to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Mail comes from a server. Only [`Incoming::Local`] has none: it keeps its mail on this
    /// computer, so it is never synced, never waited on and never sent from.
    pub has_server: bool,
    /// The account lists folders. POP3 has one mailbox and local folders have no server to
    /// list them from.
    pub lists_folders: bool,
    /// A search can be sent to the server.
    pub searchable: bool,
    /// The server can say how much storage is used.
    pub has_quota: bool,
    /// The server can be waited on for new mail (IMAP `IDLE`, JMAP push).
    pub pushes: bool,
    /// One message can be fetched on its own, without a sync.
    pub fetches_one: bool,
    /// Mailboxes are folders under an inbox that every listing has, which a sync follows. JMAP's
    /// are labels on mail fetched whole.
    pub folder_tree: bool,
    /// The client writes to the server's mailboxes: uploads a message into one (`APPEND`) and
    /// rewrites them.
    pub writes_mailboxes: bool,
    /// The server keeps the copies of drafts the client uploads to it.
    pub server_drafts: bool,
}

/// An incoming server reached over a socket: the host, port and transport security the plan
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Server<'a> {
    pub host: &'a str,
    pub port: u16,
    pub tls: Tls,
}

impl Incoming {
    /// What this protocol can do.
    pub const fn caps(&self) -> Caps {
        match self {
            Self::Imap { .. } => Caps {
                has_server: true,
                lists_folders: true,
                searchable: true,
                has_quota: true,
                pushes: true,
                fetches_one: true,
                folder_tree: true,
                writes_mailboxes: true,
                server_drafts: true,
            },
            Self::Pop3 { .. } => Caps {
                has_server: true,
                lists_folders: false,
                searchable: false,
                has_quota: false,
                pushes: false,
                fetches_one: false,
                folder_tree: false,
                writes_mailboxes: false,
                server_drafts: false,
            },
            Self::Local => Caps {
                has_server: false,
                lists_folders: false,
                searchable: false,
                has_quota: false,
                pushes: false,
                fetches_one: false,
                folder_tree: false,
                writes_mailboxes: false,
                server_drafts: false,
            },
            Self::Graph => Caps {
                has_server: true,
                lists_folders: true,
                searchable: true,
                has_quota: false,
                pushes: false,
                fetches_one: true,
                folder_tree: true,
                writes_mailboxes: false,
                server_drafts: false,
            },
            Self::Jmap { .. } => Caps {
                has_server: true,
                lists_folders: true,
                searchable: true,
                has_quota: true,
                pushes: true,
                fetches_one: false,
                folder_tree: false,
                writes_mailboxes: false,
                server_drafts: true,
            },
        }
    }

    /// The socket server mail is fetched from: IMAP's or POP3's. `None` for local folders, and
    /// for Graph and JMAP, which are HTTPS to a fixed host or a session URL and have no host and
    /// port of the plan's own to open.
    pub fn server(&self) -> Option<Server<'_>> {
        match self {
            Self::Imap { host, port, tls }
            | Self::Pop3 {
                host, port, tls, ..
            } => Some(Server {
                host,
                port: *port,
                tls: *tls,
            }),
            Self::Local | Self::Graph | Self::Jmap { .. } => None,
        }
    }

    /// The host of [`Self::server`].
    pub fn host(&self) -> Option<&str> {
        self.server().map(|server| server.host)
    }
}
