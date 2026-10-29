//! What an account's incoming protocol can do, as a table of what the old matches said.
//!
//! The stored forms are the persisted `kind`/`v` shapes, as an earlier build wrote them. The
//! answers are what each place that used to match on the protocol said, transcribed once per
//! protocol before `Incoming::caps` existed, and `Incoming::caps` is checked against them.

use mail_domain::{Incoming, Tls};

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

/// `Incoming::caps` gives the answers the table holds, which are what the old ladders said.
#[test]
fn caps_are_what_the_old_ladders_said() {
    for row in rows() {
        let i: Incoming = serde_json::from_str(row.json).unwrap();
        let caps = i.caps();
        let said = |what: &str, table: bool, got: bool| {
            assert_eq!(table, got, "{what} of {}", row.json);
        };
        said("has_server", row.has_server, caps.has_server);
        said("lists_folders", row.lists_folders, caps.lists_folders);
        said("searchable", row.searchable, caps.searchable);
        said("has_quota", row.has_quota, caps.has_quota);
        said("pushes", row.pushes, caps.pushes);
        said("fetches_one", row.fetches_one, caps.fetches_one);
        said("folder_tree", row.folder_tree, caps.folder_tree);
        said(
            "writes_mailboxes",
            row.writes_mailboxes,
            caps.writes_mailboxes,
        );
        said("server_drafts", row.server_drafts, caps.server_drafts);
        assert_eq!(
            row.socket,
            i.server().map(|server| (server.host, server.port)),
            "server of {}",
            row.json
        );
        assert_eq!(row.socket.map(|(host, _)| host), i.host());
    }
}

/// The tls mode of the socket is the plan's own.
#[test]
fn a_socket_server_carries_its_transport_security() {
    let read = |json| serde_json::from_str::<Incoming>(json).unwrap();
    assert_eq!(read(IMAP).server().unwrap().tls, Tls::Implicit);
    assert_eq!(
        read(IMAP_STARTTLS).server().unwrap().tls,
        Tls::StartTlsRequired
    );
    assert_eq!(
        read(POP3_DELETING).server().unwrap().tls,
        Tls::StartTlsRequired
    );
}

/// The host mail is served from is read from a JMAP session URL as a URL: without its port, user
/// part or brackets, in lower case, and not at all from text that is no URL.
#[test]
fn served_from_reads_a_session_url_as_a_url() {
    let jmap = |session: &str| Incoming::Jmap {
        session: session.to_owned(),
        auth: mail_domain::HttpAuth::Basic,
    };
    let cases = [
        (
            "https://mail.example.test/.well-known/jmap",
            Some("mail.example.test"),
        ),
        (
            "https://ada:pw@mail.example.test:8443/jmap",
            Some("mail.example.test"),
        ),
        (
            "https://Mail.Example.Test/jmap?x=1#top",
            Some("mail.example.test"),
        ),
        ("https://[2001:db8::1]:8443/jmap", Some("2001:db8::1")),
        ("mail.example.test/jmap", None),
        ("", None),
    ];
    for (session, want) in cases {
        assert_eq!(jmap(session).served_from().as_deref(), want, "{session:?}");
    }
    let imap = Incoming::Imap {
        host: "imap.example.test".to_owned(),
        port: 993,
        tls: Tls::Implicit,
    };
    assert_eq!(imap.served_from().as_deref(), Some("imap.example.test"));
    assert_eq!(Incoming::Graph.served_from(), None);
    assert_eq!(Incoming::Local.served_from(), None);
}
