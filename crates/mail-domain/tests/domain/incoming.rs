//! What an account's incoming protocol can do, written down as a table before anything derives it.
//!
//! The stored forms are the persisted `kind`/`v` shapes, as an earlier build wrote them. The
//! answers are what each place that used to match on the protocol said, transcribed once per
//! protocol: the predicates themselves are restated here as they stood, and the table is checked
//! against them, so the table is the old behaviour and not a guess at it.

use mail_domain::Incoming;

/// One stored form and what the old matches said of it.
struct Row {
    json: &'static str,
    /// Mail is fetched from a server (everything but `Local`).
    has_server: bool,
    /// The account lists folders: not POP3's one mailbox, not local folders.
    lists_folders: bool,
    /// A search can be sent to the server.
    searchable: bool,
    /// The server can say how much storage is used.
    has_quota: bool,
    /// The server can be waited on for new mail (IDLE, JMAP push).
    pushes: bool,
    /// One message can be fetched on its own.
    fetches_one: bool,
    /// Mailboxes are real folders, with an inbox, that a sync follows.
    folder_tree: bool,
    /// The client writes to the server's mailboxes (APPEND, rewrites).
    writes_mailboxes: bool,
    /// The server keeps the copies of drafts the client uploads.
    server_drafts: bool,
    /// The socket the client opens, if any.
    socket: Option<(&'static str, u16)>,
}

const IMAP: &str =
    r#"{"kind":"imap","v":{"host":"imap.example.test","port":993,"tls":"implicit"}}"#;
const IMAP_STARTTLS: &str =
    r#"{"kind":"imap","v":{"host":"imap.example.test","port":143,"tls":"start_tls_required"}}"#;
const POP3: &str =
    r#"{"kind":"pop3","v":{"host":"pop.example.test","port":995,"tls":"implicit","leave":"keep"}}"#;
const POP3_DELETING: &str = r#"{"kind":"pop3","v":{"host":"pop.example.test","port":110,"tls":"start_tls_required","leave":"delete_after_fetch"}}"#;
const LOCAL: &str = r#"{"kind":"local"}"#;
const GRAPH: &str = r#"{"kind":"graph"}"#;
const JMAP: &str = r#"{"kind":"jmap","v":{"session":"https://jmap.example.test/.well-known/jmap","auth":"basic"}}"#;
const JMAP_BEARER: &str = r#"{"kind":"jmap","v":{"session":"https://jmap.example.test/.well-known/jmap","auth":"bearer"}}"#;

fn rows() -> Vec<Row> {
    let imap = |json, port| Row {
        json,
        has_server: true,
        lists_folders: true,
        searchable: true,
        has_quota: true,
        pushes: true,
        fetches_one: true,
        folder_tree: true,
        writes_mailboxes: true,
        server_drafts: true,
        socket: Some(("imap.example.test", port)),
    };
    let pop3 = |json, port| Row {
        json,
        has_server: true,
        lists_folders: false,
        searchable: false,
        has_quota: false,
        pushes: false,
        fetches_one: false,
        folder_tree: false,
        writes_mailboxes: false,
        server_drafts: false,
        socket: Some(("pop.example.test", port)),
    };
    let jmap = |json| Row {
        json,
        has_server: true,
        lists_folders: true,
        searchable: true,
        has_quota: true,
        pushes: true,
        fetches_one: false,
        folder_tree: false,
        writes_mailboxes: false,
        server_drafts: true,
        socket: None,
    };
    vec![
        imap(IMAP, 993),
        imap(IMAP_STARTTLS, 143),
        pop3(POP3, 995),
        pop3(POP3_DELETING, 110),
        Row {
            json: LOCAL,
            has_server: false,
            lists_folders: false,
            searchable: false,
            has_quota: false,
            pushes: false,
            fetches_one: false,
            folder_tree: false,
            writes_mailboxes: false,
            server_drafts: false,
            socket: None,
        },
        Row {
            json: GRAPH,
            has_server: true,
            lists_folders: true,
            searchable: true,
            has_quota: false,
            pushes: false,
            fetches_one: true,
            folder_tree: true,
            writes_mailboxes: false,
            server_drafts: false,
            socket: None,
        },
        jmap(JMAP),
        jmap(JMAP_BEARER),
    ]
}

/// Every stored form reads, and writes back the same bytes.
#[test]
fn every_stored_incoming_form_round_trips_unchanged() {
    for row in rows() {
        let read: Incoming = serde_json::from_str(row.json).expect(row.json);
        assert_eq!(serde_json::to_string(&read).unwrap(), row.json);
    }
}

/// The ladders as they stood in the places that asked, one per question.
mod old {
    use mail_domain::Incoming;

    pub fn has_server(i: &Incoming) -> bool {
        !matches!(i, Incoming::Local)
    }
    pub fn lists_folders(i: &Incoming) -> bool {
        !matches!(i, Incoming::Pop3 { .. } | Incoming::Local)
    }
    pub fn searchable(i: &Incoming) -> bool {
        matches!(
            i,
            Incoming::Imap { .. } | Incoming::Jmap { .. } | Incoming::Graph
        )
    }
    pub fn has_quota(i: &Incoming) -> bool {
        matches!(i, Incoming::Imap { .. } | Incoming::Jmap { .. })
    }
    pub fn pushes(i: &Incoming) -> bool {
        matches!(i, Incoming::Imap { .. } | Incoming::Jmap { .. })
    }
    pub fn fetches_one(i: &Incoming) -> bool {
        matches!(i, Incoming::Imap { .. } | Incoming::Graph)
    }
    pub fn folder_tree(i: &Incoming) -> bool {
        matches!(i, Incoming::Imap { .. } | Incoming::Graph)
    }
    pub fn writes_mailboxes(i: &Incoming) -> bool {
        matches!(i, Incoming::Imap { .. })
    }
    pub fn server_drafts(i: &Incoming) -> bool {
        matches!(i, Incoming::Imap { .. } | Incoming::Jmap { .. })
    }
    pub fn socket(i: &Incoming) -> Option<(&str, u16)> {
        match i {
            Incoming::Imap { host, port, .. } | Incoming::Pop3 { host, port, .. } => {
                Some((host, *port))
            }
            Incoming::Local | Incoming::Graph | Incoming::Jmap { .. } => None,
        }
    }
}

/// The table says what the old ladders said.
#[test]
fn the_table_is_what_the_old_ladders_said() {
    for row in rows() {
        let i: Incoming = serde_json::from_str(row.json).unwrap();
        let said = |what: &str, table: bool, old: bool| {
            assert_eq!(table, old, "{what} of {}", row.json);
        };
        said("has_server", row.has_server, old::has_server(&i));
        said("lists_folders", row.lists_folders, old::lists_folders(&i));
        said("searchable", row.searchable, old::searchable(&i));
        said("has_quota", row.has_quota, old::has_quota(&i));
        said("pushes", row.pushes, old::pushes(&i));
        said("fetches_one", row.fetches_one, old::fetches_one(&i));
        said("folder_tree", row.folder_tree, old::folder_tree(&i));
        said(
            "writes_mailboxes",
            row.writes_mailboxes,
            old::writes_mailboxes(&i),
        );
        said("server_drafts", row.server_drafts, old::server_drafts(&i));
        assert_eq!(row.socket, old::socket(&i), "socket of {}", row.json);
    }
}
