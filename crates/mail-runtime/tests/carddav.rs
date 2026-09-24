//! CardDAV against a server in this process.
//!
//! No network. The "server" is a `TcpListener` behind TLS with the self-signed certificate in
//! `tests/fixtures/tls/` (its names are `unsubscribe.test` and `plain.test`, which is why a
//! CardDAV server answers at the first), and the client is told to trust it and to resolve that
//! name to the listener. It keeps an address book in memory and answers the requests RFC 6352,
//! RFC 6578 and RFC 6764 describe — well-known redirect, principal, home set, `sync-collection`
//! with tokens, `addressbook-multiget` and an etag `PROPFIND` — from that book, so a test can
//! change a card between syncs and watch the change arrive.

use mail_domain::{Retry, Retryable};
use mail_runtime::RuntimeError;
use mail_runtime::carddav::{self, CardDavFailure, Dav, DavAuth, How};
use mail_store::{AddressBook, MemoryStore, Origin, Store};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

const CERT: &[u8] = include_bytes!("fixtures/tls/cert.pem");
const KEY: &[u8] = include_bytes!("fixtures/tls/key.pem");
const HOST: &str = "unsubscribe.test";
/// `Basic` for `ada:secret`.
const GOOD: &str = "Basic YWRhOnNlY3JldA==";

const BOOK: &str = "/home/ada/contacts/";

/// What the fake server holds, and how it behaves.
#[derive(Debug, Default)]
struct Server {
    /// Card path to (etag, vCard).
    cards: BTreeMap<String, (String, String)>,
    /// Every change, in order, as (path, present?). A sync token is an index into this.
    log: Vec<(String, bool)>,
    /// Refuse `sync-collection`, like a server without RFC 6578.
    no_sync: bool,
    /// Requests seen, as `METHOD path`.
    seen: Vec<String>,
    /// The `Authorization` header of every request.
    auth: Vec<Option<String>>,
}

impl Server {
    fn put(&mut self, name: &str, vcard: &str) {
        let path = format!("{BOOK}{name}.vcf");
        let etag = format!("\"{}\"", self.log.len() + 1);
        self.cards.insert(path.clone(), (etag, vcard.to_owned()));
        self.log.push((path, true));
    }

    fn remove(&mut self, name: &str) {
        let path = format!("{BOOK}{name}.vcf");
        self.cards.remove(&path);
        self.log.push((path, false));
    }

    fn token(&self) -> String {
        format!("https://{HOST}/sync/{}", self.log.len())
    }

    fn answer(&mut self, method: &str, path: &str, auth: Option<&str>, body: &str) -> String {
        self.seen.push(format!("{method} {path}"));
        self.auth.push(auth.map(str::to_owned));
        if path == "/.well-known/carddav" {
            return "HTTP/1.1 301 Moved Permanently\r\nLocation: /dav/\r\nContent-Length: 0\r\n\
                    Connection: close\r\n\r\n"
                .to_owned();
        }
        if auth != Some(GOOD) {
            return status("401 Unauthorized", "");
        }
        match (method, path) {
            ("PROPFIND", "/dav/") => multistatus(&response(
                "/dav/",
                "<d:current-user-principal><d:href>/principals/ada/</d:href></d:current-user-principal>\
                 <d:resourcetype><d:collection/></d:resourcetype>",
            )),
            ("PROPFIND", "/principals/ada/") => multistatus(&response(
                "/principals/ada/",
                "<c:addressbook-home-set><d:href>/home/ada/</d:href></c:addressbook-home-set>",
            )),
            ("PROPFIND", "/home/ada/") => multistatus(&format!(
                "{}{}{}",
                response(
                    "/home/ada/",
                    "<d:resourcetype><d:collection/></d:resourcetype>"
                ),
                response(
                    BOOK,
                    "<d:resourcetype><d:collection/><c:addressbook/></d:resourcetype>\
                     <d:displayname>Contacts</d:displayname>"
                ),
                response(
                    "/home/ada/calendar/",
                    "<d:resourcetype><d:collection/><x:calendar xmlns:x=\"urn:x\"/></d:resourcetype>"
                ),
            )),
            ("PROPFIND", BOOK) => {
                let mut all = response(
                    BOOK,
                    "<d:resourcetype><d:collection/><c:addressbook/></d:resourcetype>",
                );
                for (path, (etag, _)) in &self.cards {
                    all.push_str(&response(path, &format!("<d:getetag>{etag}</d:getetag>")));
                }
                multistatus(&all)
            }
            ("REPORT", BOOK) if body.contains("sync-collection") => {
                if self.no_sync {
                    return status("403 Forbidden", "");
                }
                self.sync_report(body)
            }
            ("REPORT", BOOK) if body.contains("addressbook-multiget") => {
                let doc = roxmltree::Document::parse(body).unwrap();
                let mut all = String::new();
                for href in doc
                    .descendants()
                    .filter(|n| n.has_tag_name(("DAV:", "href")))
                {
                    let href = href.text().unwrap_or_default();
                    match self.cards.get(href) {
                        Some((etag, card)) => all.push_str(&response(
                            href,
                            &format!(
                                "<d:getetag>{etag}</d:getetag><c:address-data><![CDATA[{card}]]></c:address-data>"
                            ),
                        )),
                        None => all.push_str(&gone(href)),
                    }
                }
                multistatus(&all)
            }
            _ => status("404 Not Found", ""),
        }
    }

    /// Changes since the token in `body`, or everything for an empty one.
    fn sync_report(&self, body: &str) -> String {
        let doc = roxmltree::Document::parse(body).unwrap();
        let token = doc
            .descendants()
            .find(|n| n.has_tag_name(("DAV:", "sync-token")))
            .and_then(|n| n.text())
            .unwrap_or_default();
        let from = if token.is_empty() {
            None
        } else {
            match token
                .rsplit('/')
                .next()
                .and_then(|n| n.parse::<usize>().ok())
            {
                Some(n)
                    if token.starts_with(&format!("https://{HOST}/sync/"))
                        && n <= self.log.len() =>
                {
                    Some(n)
                }
                _ => {
                    return status(
                        "403 Forbidden",
                        "<d:error xmlns:d=\"DAV:\"><d:valid-sync-token/></d:error>",
                    );
                }
            }
        };
        let mut all = String::new();
        match from {
            None => {
                for (path, (etag, _)) in &self.cards {
                    all.push_str(&response(path, &format!("<d:getetag>{etag}</d:getetag>")));
                }
            }
            Some(n) => {
                let mut latest: BTreeMap<&str, bool> = BTreeMap::new();
                for (path, present) in &self.log[n..] {
                    latest.insert(path, *present);
                }
                for (path, present) in latest {
                    match (present, self.cards.get(path)) {
                        (true, Some((etag, _))) => {
                            all.push_str(&response(path, &format!("<d:getetag>{etag}</d:getetag>")))
                        }
                        _ => all.push_str(&gone(path)),
                    }
                }
            }
        }
        all.push_str(&format!("<d:sync-token>{}</d:sync-token>", self.token()));
        multistatus(&all)
    }
}

fn status(line: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {line}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn multistatus(inner: &str) -> String {
    status(
        "207 Multi-Status",
        &format!(
            "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\" \
             xmlns:c=\"urn:ietf:params:xml:ns:carddav\">{inner}</d:multistatus>"
        ),
    )
}

fn response(href: &str, props: &str) -> String {
    format!(
        "<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop>\
         <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
    )
}

fn gone(href: &str) -> String {
    format!(
        "<d:response><d:href>{href}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>"
    )
}

type Shared = Arc<Mutex<Server>>;

/// Start the server; every connection is one request, answered from `server`.
async fn serve(server: Shared) -> SocketAddr {
    let certs = vec![CertificateDer::from_pem_slice(CERT).unwrap()];
    let key = PrivateKeyDer::from_pem_slice(KEY).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let Ok(mut stream) = acceptor.accept(sock).await else {
                continue;
            };
            let Some((method, path, auth, body)) = read_request(&mut stream).await else {
                continue;
            };
            let reply = server
                .lock()
                .unwrap()
                .answer(&method, &path, auth.as_deref(), &body);
            let _ = stream.write_all(reply.as_bytes()).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
    });
    addr
}

async fn read_request<S: tokio::io::AsyncRead + Unpin>(
    stream: &mut S,
) -> Option<(String, String, Option<String>, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&buf).to_string();
        let Some(end) = text.find("\r\n\r\n") else {
            continue;
        };
        let head = &text[..end];
        let header = |name: &str| {
            head.lines().skip(1).find_map(|line| {
                let (n, v) = line.split_once(':')?;
                n.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
            })
        };
        let length: usize = header("content-length")
            .and_then(|l| l.parse().ok())
            .unwrap_or(0);
        if buf.len() < end + 4 + length {
            continue;
        }
        let mut words = head.lines().next()?.split_whitespace();
        let method = words.next()?.to_owned();
        let path = words.next()?.to_owned();
        let body = String::from_utf8_lossy(&buf[end + 4..end + 4 + length]).to_string();
        return Some((method, path, header("authorization"), body));
    }
}

fn dav(addr: SocketAddr, auth: DavAuth) -> Dav {
    let http = carddav::client_builder()
        .add_root_certificate(reqwest::Certificate::from_pem(CERT).unwrap())
        .resolve(HOST, addr)
        .build()
        .unwrap();
    Dav::new(http, &base(addr, "/"), auth).unwrap()
}

fn base(addr: SocketAddr, path: &str) -> Url {
    Url::parse(&format!("https://{HOST}:{}{path}", addr.port())).unwrap()
}

fn ada() -> DavAuth {
    DavAuth::Basic {
        user: "ada".to_owned(),
        password: "secret".to_owned(),
    }
}

fn card(name: &str, emails: &[&str]) -> String {
    let mut out = format!("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:{name}\r\n");
    for email in emails {
        out.push_str(&format!("EMAIL;TYPE=INTERNET:{email}\r\n"));
    }
    out.push_str("END:VCARD\r\n");
    out
}

fn names(store: &MemoryStore) -> Vec<(String, Option<String>)> {
    let mut all: Vec<_> = store
        .contacts()
        .unwrap()
        .into_iter()
        .map(|c| (c.address, c.name))
        .collect();
    all.sort();
    all
}

async fn book_at(addr: SocketAddr, store: &MemoryStore) -> AddressBook {
    let url = base(addr, BOOK).to_string();
    store.address_book(&url).unwrap().unwrap_or(AddressBook {
        url,
        ..AddressBook::default()
    })
}

#[tokio::test]
async fn discovery_follows_well_known_to_the_principal_home_and_its_address_books() {
    let server: Shared = Arc::default();
    let addr = serve(server.clone()).await;
    let found = carddav::discover(&dav(addr, ada()), &base(addr, "/"))
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "the calendar is not an address book");
    assert_eq!(found[0].url, base(addr, BOOK));
    assert_eq!(found[0].name.as_deref(), Some("Contacts"));
    let seen = server.lock().unwrap().seen.clone();
    assert_eq!(
        seen,
        [
            "PROPFIND /.well-known/carddav",
            "PROPFIND /dav/",
            "PROPFIND /principals/ada/",
            "PROPFIND /home/ada/",
        ]
    );
}

#[tokio::test]
async fn an_address_book_url_is_its_own_answer() {
    let server: Shared = Arc::default();
    let addr = serve(server.clone()).await;
    let found = carddav::discover(&dav(addr, ada()), &base(addr, BOOK))
        .await
        .unwrap();
    assert_eq!(found[0].url, base(addr, BOOK));
    assert_eq!(
        server.lock().unwrap().seen,
        ["PROPFIND /home/ada/contacts/"]
    );
}

#[tokio::test]
async fn a_sync_brings_cards_in_and_the_next_one_only_what_changed() {
    let server: Shared = Arc::default();
    {
        let mut s = server.lock().unwrap();
        s.put(
            "ada",
            &card(
                "Ada Lovelace",
                &["Ada@example.test", "ada@work.example.test"],
            ),
        );
        s.put("bob", &card("Bob", &["bob@example.test"]));
    }
    let addr = serve(server.clone()).await;
    let store = MemoryStore::new();
    let dav = dav(addr, ada());

    let first = carddav::sync(&dav, &store, book_at(addr, &store).await)
        .await
        .unwrap();
    assert_eq!((first.changed, first.removed, first.how), (2, 0, How::Full));
    assert_eq!(
        names(&store),
        [
            (
                "ada@example.test".to_owned(),
                Some("Ada Lovelace".to_owned())
            ),
            (
                "ada@work.example.test".to_owned(),
                Some("Ada Lovelace".to_owned())
            ),
            ("bob@example.test".to_owned(), Some("Bob".to_owned())),
        ]
    );
    let source = format!("carddav:{}", base(addr, BOOK));
    assert_eq!(
        store.contact("bob@example.test").unwrap().unwrap().origin,
        Origin::Book(source)
    );

    // Ada drops her work address and Bob goes.
    {
        let mut s = server.lock().unwrap();
        s.put("ada", &card("Ada King", &["ada@example.test"]));
        s.remove("bob");
        s.seen.clear();
    }
    let second = carddav::sync(&dav, &store, book_at(addr, &store).await)
        .await
        .unwrap();
    assert_eq!(
        (second.changed, second.removed, second.how),
        (1, 1, How::Incremental)
    );
    assert_eq!(
        names(&store),
        [("ada@example.test".to_owned(), Some("Ada King".to_owned()))]
    );
    let seen = server.lock().unwrap().seen.clone();
    assert_eq!(
        seen,
        ["REPORT /home/ada/contacts/", "REPORT /home/ada/contacts/"],
        "one sync report and one multiget of the one changed card"
    );

    // Nothing changed: nothing fetched.
    server.lock().unwrap().seen.clear();
    let third = carddav::sync(&dav, &store, book_at(addr, &store).await)
        .await
        .unwrap();
    assert_eq!((third.changed, third.removed), (0, 0));
    assert_eq!(server.lock().unwrap().seen.len(), 1);
}

#[tokio::test]
async fn a_token_the_server_no_longer_honours_starts_the_sync_over() {
    let server: Shared = Arc::default();
    server
        .lock()
        .unwrap()
        .put("ada", &card("Ada", &["ada@example.test"]));
    let addr = serve(server.clone()).await;
    let store = MemoryStore::new();
    let mut book = book_at(addr, &store).await;
    book.token = Some("https://elsewhere.test/sync/99".to_owned());
    let synced = carddav::sync(&dav(addr, ada()), &store, book)
        .await
        .unwrap();
    assert_eq!((synced.changed, synced.how), (1, How::Full));
    assert!(store.contact("ada@example.test").unwrap().is_some());
}

#[tokio::test]
async fn a_server_without_sync_collection_is_synced_by_etags() {
    let server: Shared = Arc::default();
    {
        let mut s = server.lock().unwrap();
        s.no_sync = true;
        s.put("ada", &card("Ada", &["ada@example.test"]));
        s.put("bob", &card("Bob", &["bob@example.test"]));
    }
    let addr = serve(server.clone()).await;
    let store = MemoryStore::new();
    let dav = dav(addr, ada());
    let first = carddav::sync(&dav, &store, book_at(addr, &store).await)
        .await
        .unwrap();
    assert_eq!((first.changed, first.how), (2, How::Etags));

    {
        let mut s = server.lock().unwrap();
        s.put("bob", &card("Robert", &["bob@example.test"]));
        s.remove("ada");
    }
    let second = carddav::sync(&dav, &store, book_at(addr, &store).await)
        .await
        .unwrap();
    assert_eq!((second.changed, second.removed), (1, 1));
    assert_eq!(
        names(&store),
        [("bob@example.test".to_owned(), Some("Robert".to_owned()))]
    );
}

#[tokio::test]
async fn a_card_leaving_the_book_hands_an_address_back_to_mail_and_spares_a_hand_edit() {
    use mail_domain::*;
    let server: Shared = Arc::default();
    {
        let mut s = server.lock().unwrap();
        s.put("ada", &card("Ada (book)", &["ada@example.test"]));
        s.put("zed", &card("Zed (book)", &["zed@example.test"]));
    }
    let addr = serve(server.clone()).await;
    let store = MemoryStore::new();
    // Zed was typed in by hand before the book knew him; Ada had written to the user.
    store
        .put_contact("zed@example.test", Some("Zed"), &Origin::Manual)
        .unwrap();
    let account = AccountId::from_uuid(uuid::Uuid::from_u128(1));
    let message = Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(2)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(3)),
        account,
        key: MessageKey::Rfc("m@example.test".to_owned()),
        date: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        from: Address {
            name: Some("Ada L.".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "hi".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    store
        .ingest(
            account,
            Ingest {
                mailbox: MailboxRef {
                    account,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: "1".to_owned(),
                    },
                    key: message.key.clone(),
                    raw: BlobId::generate(),
                    message: message.clone(),
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();

    let dav = dav(addr, ada());
    carddav::sync(&dav, &store, book_at(addr, &store).await)
        .await
        .unwrap();
    assert_eq!(
        store
            .contact("ada@example.test")
            .unwrap()
            .unwrap()
            .name
            .as_deref(),
        Some("Ada (book)")
    );
    assert_eq!(
        store
            .contact("zed@example.test")
            .unwrap()
            .unwrap()
            .name
            .as_deref(),
        Some("Zed"),
        "a hand edit outranks the book"
    );

    {
        let mut s = server.lock().unwrap();
        s.remove("ada");
        s.remove("zed");
    }
    carddav::sync(&dav, &store, book_at(addr, &store).await)
        .await
        .unwrap();
    let ada = store.contact("ada@example.test").unwrap().unwrap();
    assert_eq!(
        ada.origin,
        Origin::History,
        "mail knew her before the book did"
    );
    assert_eq!(ada.received.count, 1);
    assert!(
        store.contact("zed@example.test").unwrap().is_some(),
        "the book never owned Zed"
    );
}

#[tokio::test]
async fn a_refused_password_is_a_reauth_and_the_book_is_left_as_it_was() {
    let server: Shared = Arc::default();
    server
        .lock()
        .unwrap()
        .put("ada", &card("Ada", &["ada@example.test"]));
    let addr = serve(server.clone()).await;
    let store = MemoryStore::new();
    let wrong = DavAuth::Basic {
        user: "ada".to_owned(),
        password: "wrong".to_owned(),
    };
    let err = carddav::sync(&dav(addr, wrong), &store, book_at(addr, &store).await)
        .await
        .unwrap_err();
    assert!(
        matches!(err, RuntimeError::CardDav(CardDavFailure::Unauthorized)),
        "{err:?}"
    );
    assert_eq!(err.retry(), Retry::NeedsReauth);
    assert_eq!(store.contacts().unwrap(), vec![]);
    assert_eq!(store.address_book(base(addr, BOOK).as_str()).unwrap(), None);
}

#[tokio::test]
async fn a_bearer_token_is_presented_as_one() {
    let server: Shared = Arc::default();
    let addr = serve(server.clone()).await;
    let _ = carddav::discover(
        &dav(addr, DavAuth::Bearer("tok".to_owned())),
        &base(addr, BOOK),
    )
    .await;
    let auth = server.lock().unwrap().auth.clone();
    assert_eq!(auth, [Some("Bearer tok".to_owned())]);
}

#[test]
fn a_credential_is_never_in_debug_output() {
    let shown = format!("{:?} {:?}", ada(), DavAuth::Bearer("tok".to_owned()));
    assert!(
        !shown.contains("secret") && !shown.contains("tok\""),
        "{shown}"
    );
}

#[test]
fn an_http_url_is_refused_before_anything_is_sent() {
    let err = Dav::new(
        carddav::client().unwrap(),
        &Url::parse("http://dav.example.test/").unwrap(),
        ada(),
    )
    .unwrap_err();
    assert!(
        matches!(err, RuntimeError::CardDav(CardDavFailure::Insecure(_))),
        "{err:?}"
    );
}
