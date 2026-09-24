//! Reading a mailbox through Microsoft Graph, for a Microsoft 365 tenant that switched IMAP off.
//!
//! Graph rather than Exchange Web Services, which Exchange Online retires from 2026-10-01. The
//! [`Reader`] answers the same [`ProtoOp`]s an IMAP backend does, over HTTPS instead of a
//! session, so the engine's passes, its outbox, its renewal and its rules serve a Graph account
//! unchanged:
//!
//! - **Folders** come from `GET /me/mailFolders` and each folder's `childFolders`, depth first
//!   and bounded. A folder's path is its display names joined by `/`, with the inbox called
//!   `INBOX` as everywhere else; Graph's ids are looked up from the path when a request needs
//!   one. The well-known folders (`inbox`, `sentitems`, `drafts`, `deleteditems`, `junkemail`,
//!   `archive`) are asked for by name, which is how their roles are found in any language.
//! - **Sync** is a delta query per folder. Its first request names the properties wanted; every
//!   later one is the `@odata.nextLink` or `@odata.deltaLink` Graph returned, stored as the
//!   folder's [`SyncCursor::Graph`]. An entry marked `@removed` has left the folder. A pass
//!   follows pages until it has seen its share of messages and stores the `nextLink` it reached,
//!   so a first sync of a large folder is spread over passes like an IMAP backfill is.
//! - **Headers** for the list are written from the delta's own properties — `From`, `To`, `Cc`,
//!   `Subject`, `Date`, `Message-ID`, and `In-Reply-To`/`References` from the MAPI properties
//!   Exchange keeps them in — so a new message is listed and threaded without another request.
//! - **Bodies** are `GET /me/messages/{id}/$value`: the message's own MIME, so everything
//!   downstream parses real RFC 5322 bytes. The message size (`PidTagMessageSize`) comes with the
//!   delta, and the engine fetches small bodies first, as it does over IMAP.
//! - **Changes** made here go back as `PATCH isRead`, `PATCH flag`, and `POST …/move`. A move
//!   gives the message a new id, which the move's answer names; the engine remaps it.
//!
//! Labels stay on this computer. Outlook's categories look like labels and are not the thing
//! this client means by one: [`mail_domain::ServerLabels::Supported`] says a folder *is* a label,
//! which is Gmail's model and drives folder listing and filing. Categories would need a third
//! kind of server label, and syncing them is left for when one exists.
//!
//! Nothing is ever deleted for good: `Expunge` is refused, and so is deleting a folder.
//!
//! There is no push. Graph's change notifications are delivered to a public HTTPS webhook,
//! which a desktop client does not have, so an account that reads through Graph polls — every
//! minute, which is cheap because a delta query with nothing to report returns nothing.

use crate::RuntimeError;
use crate::graph::{detail, retry_after, segment};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, FetchSince, Folder, FolderRoles, FolderWork, Holds, Ingest,
    MailboxRole, ProtoOp, ReadState, RemoteRef, Retry, SpecialUse, Star, Subscription, SyncCursor,
    UidValidity,
};
use mail_proto::ProtoOutcome;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

/// The signed-in user's mailbox, on Graph's own host.
pub const ME: &str = "https://graph.microsoft.com/v1.0/me";

/// Messages per delta page, asked for with `Prefer: odata.maxpagesize`.
const PAGE: usize = 100;

/// How many messages one delta walk goes through before it stops and keeps its place.
const PER_PASS: usize = 500;

/// Most folders listed, and deepest nesting followed. A listing is not where a mailbox with ten
/// thousand folders should find out it has them.
const MAX_FOLDERS: usize = 2000;
const MAX_DEPTH: usize = 12;

/// What a delta asks for about each message.
const SELECT: &str = "internetMessageId,receivedDateTime,sentDateTime,isRead,flag,\
    parentFolderId,subject,from,toRecipients,ccRecipients,hasAttachments,conversationId,\
    bodyPreview";

/// MAPI properties that come with each message: its size, and the two threading headers Graph
/// has no property for.
const SIZE_TAG: u32 = 0x0E08;
const IN_REPLY_TO_TAG: u32 = 0x1042;
const REFERENCES_TAG: u32 = 0x1039;
const EXPAND: &str = "singleValueExtendedProperties($filter=id eq 'Integer 0x0E08' or \
    id eq 'String 0x1042' or id eq 'String 0x1039')";

/// Graph's names for the folders a role lives in. Every mailbox answers to these, whatever
/// language its display names are in.
const WELL_KNOWN: [(&str, MailboxRole); 6] = [
    ("inbox", MailboxRole::Inbox),
    ("sentitems", MailboxRole::Sent),
    ("drafts", MailboxRole::Drafts),
    ("deleteditems", MailboxRole::Trash),
    ("junkemail", MailboxRole::Spam),
    ("archive", MailboxRole::Archive),
];

/// The inbox, by the name every protocol here gives it.
const INBOX: &str = "INBOX";

/// One account's mailbox, reached through Graph.
#[derive(Debug)]
pub struct Reader {
    /// `…/v1.0/me`: Graph's, except under test.
    me: String,
    http: reqwest::Client,
    account: AccountId,
    caps: AccountCaps,
    /// Graph's id for each folder path, from the last listing.
    folders: HashMap<String, String>,
    /// A size for each message a delta described this session, for fetching small ones first.
    sizes: HashMap<RemoteRef, u64>,
    /// Headers written from the last delta, for the engine to store.
    arrivals: Vec<(RemoteRef, Vec<u8>)>,
    /// Moves made by the last operation: where each message was, and where it is now.
    moves: Vec<(RemoteRef, RemoteRef)>,
    /// How many messages the next delta walk may go through.
    per_pass: usize,
}

impl Reader {
    /// A reader for `account`, believed to support `caps`, at Graph's own address.
    pub fn new(account: AccountId, caps: AccountCaps) -> Result<Self, RuntimeError> {
        Ok(Self {
            me: ME.to_owned(),
            http: crate::signin::http_client()?,
            account,
            caps,
            folders: HashMap::new(),
            sizes: HashMap::new(),
            arrivals: Vec::new(),
            moves: Vec::new(),
            per_pass: PER_PASS,
        })
    }

    /// Read from `me` instead of Graph: a test's own listener.
    pub fn at(mut self, me: impl Into<String>) -> Self {
        self.me = me.into();
        self
    }

    /// What this reader believes the mailbox supports.
    pub fn caps(&self) -> &AccountCaps {
        &self.caps
    }

    /// Date the capabilities: they were confirmed at `now`.
    pub fn observed(&mut self, now: DateTime<Utc>) -> AccountCaps {
        self.caps.observed_at = now;
        self.caps.clone()
    }

    /// Let the next delta walk go through at most `messages` before it stops.
    pub fn limit(&mut self, messages: usize) {
        self.per_pass = messages.max(1);
    }

    /// Every message a delta described this session, with its size where Graph gave one.
    pub fn surveyed(&self) -> Vec<(RemoteRef, u64)> {
        self.sizes.iter().map(|(r, s)| (r.clone(), *s)).collect()
    }

    /// The headers the last delta wrote, each once.
    pub fn take_arrivals(&mut self) -> Vec<(RemoteRef, Vec<u8>)> {
        std::mem::take(&mut self.arrivals)
    }

    /// The moves the last operation made, each once.
    pub fn take_moves(&mut self) -> Vec<(RemoteRef, RemoteRef)> {
        std::mem::take(&mut self.moves)
    }

    /// Do `op` with the access token `token`. `staged` is the message an `Append` uploads.
    pub async fn run(
        &mut self,
        op: ProtoOp,
        token: &str,
        staged: Option<Vec<u8>>,
    ) -> Result<ProtoOutcome, RuntimeError> {
        match op {
            ProtoOp::FetchCaps => Ok(ProtoOutcome::Caps(Box::new(self.caps.clone()))),
            ProtoOp::ListFolders => {
                let listed = self.list(token).await?;
                Ok(ProtoOutcome::Folders {
                    caps: Box::new(self.caps.clone()),
                    listed,
                })
            }
            ProtoOp::FetchEnvelopes { mailbox, since } => {
                let ingest = self.delta(mailbox, since, token).await?;
                Ok(ProtoOutcome::Ingested(Box::new(ingest)))
            }
            // The delta wrote them already; asked again, they are what it wrote.
            ProtoOp::FetchHeaders { remotes } => {
                let items = self
                    .arrivals
                    .iter()
                    .filter(|(r, _)| remotes.contains(r))
                    .cloned()
                    .collect();
                Ok(ProtoOutcome::Fetched {
                    items,
                    flags: Vec::new(),
                })
            }
            ProtoOp::FetchBody { remotes } => {
                let mut items = Vec::new();
                for remote in remotes {
                    let RemoteRef::Graph { id, .. } = &remote else {
                        continue;
                    };
                    let url = format!("{}/messages/{}/$value", self.me, segment(id));
                    match self.call(reqwest::Method::GET, &url, token, None).await? {
                        Answer::Done(bytes) => items.push((remote, bytes)),
                        // Moved or deleted since it was listed: the next delta says which.
                        Answer::Gone => {}
                    }
                }
                Ok(ProtoOutcome::Fetched {
                    items,
                    flags: Vec::new(),
                })
            }
            ProtoOp::SetFlags {
                remotes,
                read,
                star,
            } => {
                let mut patch = json!({});
                if let Some(read) = read {
                    patch["isRead"] = json!(read == ReadState::Read);
                }
                if let Some(star) = star {
                    let status = match star {
                        Star::Starred => "flagged",
                        Star::Unstarred => "notFlagged",
                    };
                    patch["flag"] = json!({ "flagStatus": status });
                }
                for id in graph_ids(&remotes) {
                    let url = format!("{}/messages/{}", self.me, segment(id));
                    self.call(reqwest::Method::PATCH, &url, token, Some(&patch))
                        .await?;
                }
                Ok(ProtoOutcome::Applied)
            }
            ProtoOp::SetMailbox { remotes, role } => {
                let destination = well_known(role).to_owned();
                let path = self
                    .caps
                    .folders
                    .path(role)
                    .map(str::to_owned)
                    .unwrap_or_else(|| match role {
                        MailboxRole::Inbox => INBOX.to_owned(),
                        _ => destination.clone(),
                    });
                self.move_all(&remotes, &destination, &path, token).await?;
                Ok(ProtoOutcome::Applied)
            }
            ProtoOp::File { remotes, folder } => {
                let destination = self.folder_id(&folder, token).await?;
                self.move_all(&remotes, &destination, &folder, token)
                    .await?;
                Ok(ProtoOutcome::Applied)
            }
            // Labels and keywords stay here; see the module's documentation.
            ProtoOp::SetLabels { .. } | ProtoOp::AddKeyword { .. } => Ok(ProtoOutcome::Applied),
            ProtoOp::Append { mailbox, .. } => {
                let raw = staged.ok_or_else(|| refused_here("an upload with no message"))?;
                self.upload_draft(&mailbox.path, &raw, token).await
            }
            ProtoOp::Folder(work) => self.folder_work(work, token).await,
            ProtoOp::Expunge { .. } => Err(refused_here(
                "deleting mail for good; this client never does that through Microsoft Graph",
            )),
            ProtoOp::Watch { .. } => Ok(ProtoOutcome::Applied),
            ProtoOp::Submit { .. }
            | ProtoOp::FetchFlags { .. }
            | ProtoOp::ListRemote { .. }
            | ProtoOp::FetchStructure { .. }
            | ProtoOp::FetchSections { .. } => Err(RuntimeError::UnsupportedIo(
                "Microsoft Graph's delta query answers this; it is not asked separately".to_owned(),
            )),
        }
    }

    /// Follow one folder's delta from `since`, up to this pass's share of messages.
    async fn delta(
        &mut self,
        mailbox: mail_domain::MailboxRef,
        since: FetchSince,
        token: &str,
    ) -> Result<Ingest, RuntimeError> {
        let path = mailbox.path.clone();
        let first = self.first_delta(&path, token).await?;
        let (mut url, mut resumed) = match since {
            FetchSince::After {
                cursor: SyncCursor::Graph { delta_link },
            } => (delta_link, true),
            _ => (first.clone(), false),
        };
        let mut validity = UidValidity::Same;
        let mut ingest = Ingest {
            mailbox,
            validity: UidValidity::Same,
            cursor: None,
            messages: Vec::new(),
            flags: Vec::new(),
            labels: Vec::new(),
            label_names: Vec::new(),
            gone: Vec::new(),
        };
        self.arrivals.clear();
        let mut seen = 0usize;
        // Each page names the next; a server that never stops naming one must not keep this here.
        for _ in 0..10_000 {
            let page = match self.page(&url, token).await? {
                Some(page) => page,
                // The link has expired: Graph wants the folder synced again from the start.
                // Every address held for it is then stale, and the walk starts over.
                None if resumed => {
                    resumed = false;
                    validity = UidValidity::Reset;
                    url = first.clone();
                    seen = 0;
                    ingest.flags.clear();
                    ingest.gone.clear();
                    self.arrivals.clear();
                    continue;
                }
                None => {
                    return Err(RuntimeError::Graph {
                        why: format!("Microsoft Graph will not start a delta of {path}"),
                        retry: Retry::After(Duration::from_secs(300)),
                    });
                }
            };
            for item in page
                .get("value")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let remote = RemoteRef::Graph {
                    mailbox: path.clone(),
                    id: id.to_owned(),
                };
                seen += 1;
                if item.get("@removed").is_some() {
                    self.sizes.remove(&remote);
                    ingest.gone.push(remote);
                    continue;
                }
                let described = describe(item);
                self.sizes.insert(remote.clone(), described.size);
                ingest
                    .flags
                    .push((remote.clone(), described.read, described.star));
                self.arrivals.push((remote, described.headers));
            }
            if let Some(end) = page.get("@odata.deltaLink").and_then(Value::as_str) {
                ingest.cursor = Some(SyncCursor::Graph {
                    delta_link: end.to_owned(),
                });
                break;
            }
            let Some(next) = page.get("@odata.nextLink").and_then(Value::as_str) else {
                // Neither link: nothing to resume from, so the next pass starts where this did.
                ingest.cursor = None;
                break;
            };
            url = next.to_owned();
            // Kept, so a pass that stops here goes on from here rather than from the start.
            ingest.cursor = Some(SyncCursor::Graph {
                delta_link: url.clone(),
            });
            if seen >= self.per_pass {
                break;
            }
        }
        ingest.validity = validity;
        Ok(ingest)
    }

    /// The first request of a delta over `path`, naming everything the walk wants.
    async fn first_delta(&mut self, path: &str, token: &str) -> Result<String, RuntimeError> {
        let folder = self.folder_id(path, token).await?;
        let mut url = url::Url::parse(&format!(
            "{}/mailFolders/{}/messages/delta",
            self.me,
            segment(&folder)
        ))
        .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
        url.query_pairs_mut()
            .append_pair("$select", SELECT)
            .append_pair("$expand", EXPAND)
            // Newest first, so the first pass over a large folder fetches this week's mail.
            .append_pair("$orderby", "receivedDateTime desc");
        Ok(url.to_string())
    }

    /// One delta page, or `None` where Graph says the link has expired (`410 Gone`).
    async fn page(&self, url: &str, token: &str) -> Result<Option<Value>, RuntimeError> {
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .header("Prefer", format!("odata.maxpagesize={PAGE}"))
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
        let status = response.status().as_u16();
        if status == 410 {
            return Ok(None);
        }
        if !response.status().is_success() {
            let after = retry_after(&response);
            let text = response.text().await.unwrap_or_default();
            return Err(refused(status, &text, after));
        }
        let text = response
            .text()
            .await
            .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
        serde_json::from_str(&text).map(Some).map_err(|e| {
            RuntimeError::Proto(mail_proto::ProtoError::Malformed(format!(
                "Microsoft Graph's delta page: {e}"
            )))
        })
    }

    /// Move each Graph message in `remotes` to `destination`, remembering where each went.
    async fn move_all(
        &mut self,
        remotes: &[RemoteRef],
        destination: &str,
        path: &str,
        token: &str,
    ) -> Result<(), RuntimeError> {
        for remote in remotes {
            let RemoteRef::Graph { id, .. } = remote else {
                continue;
            };
            let url = format!("{}/messages/{}/move", self.me, segment(id));
            let body = json!({ "destinationId": destination });
            let Answer::Done(bytes) = self
                .call(reqwest::Method::POST, &url, token, Some(&body))
                .await?
            else {
                continue;
            };
            let moved: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            if let Some(new) = moved.get("id").and_then(Value::as_str) {
                let to = RemoteRef::Graph {
                    mailbox: path.to_owned(),
                    id: new.to_owned(),
                };
                if let Some(size) = self.sizes.remove(remote) {
                    self.sizes.insert(to.clone(), size);
                }
                self.moves.push((remote.clone(), to));
            }
        }
        Ok(())
    }

    /// Upload `raw` as a draft. Graph creates a message from MIME only as a draft, in Drafts, so
    /// that is the one place an upload can go.
    async fn upload_draft(
        &mut self,
        path: &str,
        raw: &[u8],
        token: &str,
    ) -> Result<ProtoOutcome, RuntimeError> {
        let drafts = self
            .caps
            .folders
            .path(MailboxRole::Drafts)
            .unwrap_or("drafts");
        if path != drafts && path != "drafts" {
            return Err(refused_here(
                "uploading into a folder other than Drafts; Microsoft Graph creates a message \
                 from MIME only as a draft",
            ));
        }
        let path = drafts.to_owned();
        let body = base64::engine::general_purpose::STANDARD.encode(raw);
        let response = self
            .http
            .post(format!("{}/messages", self.me))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(body)
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            let after = retry_after(&response);
            let text = response.text().await.unwrap_or_default();
            return Err(refused(status, &text, after));
        }
        let text = response.text().await.unwrap_or_default();
        let created: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let remote = created
            .get("id")
            .and_then(Value::as_str)
            .map(|id| RemoteRef::Graph {
                mailbox: path,
                id: id.to_owned(),
            });
        Ok(ProtoOutcome::Appended { remote })
    }

    /// Create or rename a folder. Deleting one is refused: Graph moves a deleted folder, and
    /// everything in it, to Deleted Items, which is further than this client goes.
    async fn folder_work(
        &mut self,
        work: FolderWork,
        token: &str,
    ) -> Result<ProtoOutcome, RuntimeError> {
        match work {
            FolderWork::Create { path } => {
                let (parent, name) = split_path(&path);
                let url = match parent {
                    Some(parent) => {
                        let parent = self.folder_id(parent, token).await?;
                        format!("{}/mailFolders/{}/childFolders", self.me, segment(&parent))
                    }
                    None => format!("{}/mailFolders", self.me),
                };
                let body = json!({ "displayName": name });
                self.call(reqwest::Method::POST, &url, token, Some(&body))
                    .await?;
            }
            FolderWork::Rename { from, to } => {
                let (old_parent, _) = split_path(&from);
                let (new_parent, name) = split_path(&to);
                if old_parent != new_parent {
                    return Err(refused_here(
                        "moving a folder under another one; rename it where it is",
                    ));
                }
                let id = self.folder_id(&from, token).await?;
                let url = format!("{}/mailFolders/{}", self.me, segment(&id));
                let body = json!({ "displayName": name });
                self.call(reqwest::Method::PATCH, &url, token, Some(&body))
                    .await?;
            }
            FolderWork::Delete { path, .. } => {
                return Err(refused_here(&format!(
                    "deleting {path}; delete it in Outlook, where it goes to Deleted Items"
                )));
            }
            // Graph has no subscriptions: every folder is listed, and following one is this
            // client's own choice.
            FolderWork::Subscribe { .. } => {}
        }
        self.folders.clear();
        Ok(ProtoOutcome::Applied)
    }

    /// Graph's id for the folder at `path`, listing the folders if it is not known yet.
    async fn folder_id(&mut self, path: &str, token: &str) -> Result<String, RuntimeError> {
        if path.eq_ignore_ascii_case(INBOX) {
            return Ok("inbox".to_owned());
        }
        if let Some(id) = self.folders.get(path) {
            return Ok(id.clone());
        }
        // A well-known name, as a path from before the first listing may be.
        if WELL_KNOWN.iter().any(|(name, _)| *name == path) {
            return Ok(path.to_owned());
        }
        self.list(token).await?;
        self.folders
            .get(path)
            .cloned()
            .ok_or_else(|| RuntimeError::Graph {
                why: format!("Microsoft Graph lists no folder called {path}"),
                retry: Retry::Fatal(format!("no folder called {path}")),
            })
    }

    /// Every folder, with the roles of the well-known ones folded into the capabilities.
    async fn list(&mut self, token: &str) -> Result<Vec<Folder>, RuntimeError> {
        // The well-known folders first, so their children are named under them.
        let mut roles: HashMap<String, MailboxRole> = HashMap::new();
        for (name, role) in WELL_KNOWN {
            let url = format!("{}/mailFolders/{name}?$select=id", self.me);
            if let Answer::Done(bytes) = self.call(reqwest::Method::GET, &url, token, None).await?
                && let Some(id) = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_owned))
            {
                roles.insert(id, role);
            }
        }

        let mut listed: Vec<(String, String)> = Vec::new();
        // Depth first, by an explicit stack: (parent path, url, depth).
        let mut pending = vec![(
            None::<String>,
            format!("{}/mailFolders?$top=100", self.me),
            0,
        )];
        while let Some((parent, first, depth)) = pending.pop() {
            let mut next = Some(first);
            while let Some(url) = next.take() {
                let Answer::Done(bytes) =
                    self.call(reqwest::Method::GET, &url, token, None).await?
                else {
                    break;
                };
                let page: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                for folder in page
                    .get("value")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let (Some(id), Some(name)) = (
                        folder.get("id").and_then(Value::as_str),
                        folder.get("displayName").and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    if folder.get("isHidden").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    let path = match (&parent, roles.get(id)) {
                        (None, Some(MailboxRole::Inbox)) => INBOX.to_owned(),
                        (Some(parent), _) => format!("{parent}/{name}"),
                        (None, _) => name.to_owned(),
                    };
                    let children = folder
                        .get("childFolderCount")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    if children > 0 && depth < MAX_DEPTH {
                        pending.push((
                            Some(path.clone()),
                            format!(
                                "{}/mailFolders/{}/childFolders?$top=100",
                                self.me,
                                segment(id)
                            ),
                            depth + 1,
                        ));
                    }
                    listed.push((path, id.to_owned()));
                    if listed.len() >= MAX_FOLDERS {
                        pending.clear();
                        break;
                    }
                }
                next = page
                    .get("@odata.nextLink")
                    .and_then(Value::as_str)
                    .filter(|_| listed.len() < MAX_FOLDERS)
                    .map(str::to_owned);
            }
        }

        self.folders = listed.iter().cloned().collect();
        let mut folder_roles: Vec<(String, MailboxRole)> = Vec::new();
        let folders = listed
            .into_iter()
            .map(|(path, id)| {
                let role = roles.get(&id).copied();
                if let Some(role) = role {
                    folder_roles.push((path.clone(), role));
                }
                Folder {
                    account: self.account,
                    path,
                    delimiter: Some('/'),
                    special: role.map(special),
                    subscription: Subscription::Subscribed,
                    holds: Holds::Mail,
                }
            })
            .collect();
        self.caps.folders = FolderRoles(folder_roles);
        if let Some(archive) = self.caps.folders.path(MailboxRole::Archive) {
            self.caps.archive = mail_domain::ArchiveMeans::MoveToFolder(archive.to_owned());
        }
        Ok(folders)
    }

    /// One request whose answer is its body; a message or folder that is not there is
    /// [`Answer::Gone`], which each caller decides about.
    async fn call(
        &self,
        method: reqwest::Method,
        url: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Result<Answer, RuntimeError> {
        let request = self.http.request(method, url).bearer_auth(token);
        let request = match body {
            Some(body) => request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(body).unwrap_or_default()),
            None => request,
        };
        let response = request
            .send()
            .await
            .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
        let status = response.status().as_u16();
        if status == 404 {
            return Ok(Answer::Gone);
        }
        if !response.status().is_success() {
            let after = retry_after(&response);
            let text = response.text().await.unwrap_or_default();
            return Err(refused(status, &text, after));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
        Ok(Answer::Done(bytes.to_vec()))
    }
}

/// What a request came back with.
enum Answer {
    Done(Vec<u8>),
    /// `404`: nothing there. A message moved or deleted since it was listed, most often.
    Gone,
}

/// Graph's ids among `remotes`.
fn graph_ids(remotes: &[RemoteRef]) -> impl Iterator<Item = &str> {
    remotes.iter().filter_map(|r| match r {
        RemoteRef::Graph { id, .. } => Some(id.as_str()),
        _ => None,
    })
}

/// Graph's well-known folder for a role.
fn well_known(role: MailboxRole) -> &'static str {
    WELL_KNOWN
        .iter()
        .find(|(_, r)| *r == role)
        .map_or("inbox", |(name, _)| name)
}

fn special(role: MailboxRole) -> SpecialUse {
    match role {
        MailboxRole::Inbox => SpecialUse::Inbox,
        MailboxRole::Archive => SpecialUse::Archive,
        MailboxRole::Sent => SpecialUse::Sent,
        MailboxRole::Drafts => SpecialUse::Drafts,
        MailboxRole::Trash => SpecialUse::Trash,
        MailboxRole::Spam => SpecialUse::Junk,
    }
}

/// `Projects/2026` as its parent and its own name.
fn split_path(path: &str) -> (Option<&str>, &str) {
    match path.rsplit_once('/') {
        Some((parent, name)) => (Some(parent), name),
        None => (None, path),
    }
}

/// Something this client will not ask of Graph. Nothing about it changes by trying again.
fn refused_here(what: &str) -> RuntimeError {
    RuntimeError::Graph {
        why: format!("not done through Microsoft Graph: {what}"),
        retry: Retry::Fatal(what.to_owned()),
    }
}

/// What a refusal of a read or a change means.
///
/// The same classes as sending's (see `graph::refusal`), with reading's own permission named:
/// throttling (`429`, and `503`/`504` under load) waits as long as `Retry-After` says.
fn refused(status: u16, body: &str, after: Option<Duration>) -> RuntimeError {
    let why = format!("Microsoft Graph refused ({status}): {}", detail(body));
    let retry = match status {
        401 => Retry::NeedsReauth,
        403 => Retry::Fatal(
            "the sign-in does not carry Graph's Mail.ReadWrite permission; re-run \
             `mailo account add … --microsoft --receive graph` and accept it, or ask an \
             administrator to consent to it"
                .to_owned(),
        ),
        429 | 503 | 504 => Retry::After(after.unwrap_or(Duration::from_secs(60))),
        500..=599 => Retry::After(Duration::from_secs(60)),
        _ => Retry::Fatal(format!("Graph answered {status}")),
    };
    RuntimeError::Graph { why, retry }
}

/// What one delta entry says about a message.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Described {
    /// Its header, written from the entry's properties.
    headers: Vec<u8>,
    read: ReadState,
    star: Star,
    /// `PidTagMessageSize`, or `u64::MAX` — unknown, and fetched last — without it.
    size: u64,
}

/// Read one delta entry.
fn describe(item: &Value) -> Described {
    let text = |name: &str| item.get(name).and_then(Value::as_str);
    let property = |tag: u32| property(item, tag);

    let mut h = String::new();
    if let Some(from) = item.get("from") {
        header(&mut h, "From", &recipients(std::slice::from_ref(from)));
    }
    for (name, field) in [("To", "toRecipients"), ("Cc", "ccRecipients")] {
        let list = item
            .get(field)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if !list.is_empty() {
            header(&mut h, name, &recipients(list));
        }
    }
    if let Some(subject) = text("subject") {
        header(&mut h, "Subject", &encoded(subject));
    }
    let date = text("sentDateTime")
        .or_else(|| text("receivedDateTime"))
        .and_then(|d| DateTime::parse_from_rfc3339(d).ok());
    if let Some(date) = date {
        header(&mut h, "Date", &date.to_rfc2822());
    }
    if let Some(id) = text("internetMessageId") {
        header(&mut h, "Message-ID", id);
    }
    if let Some(parent) = property(IN_REPLY_TO_TAG) {
        header(&mut h, "In-Reply-To", parent);
    }
    if let Some(references) = property(REFERENCES_TAG) {
        header(&mut h, "References", references);
    }
    h.push_str("\r\n");

    let read = match item.get("isRead").and_then(Value::as_bool) {
        Some(true) => ReadState::Read,
        _ => ReadState::Unread,
    };
    let star = match item
        .get("flag")
        .and_then(|f| f.get("flagStatus"))
        .and_then(Value::as_str)
    {
        Some("flagged") => Star::Starred,
        _ => Star::Unstarred,
    };
    let size = property(SIZE_TAG)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(u64::MAX);
    Described {
        headers: h.into_bytes(),
        read,
        star,
        size,
    }
}

/// The value of the extended property with `tag` on a delta entry.
fn property(item: &Value, tag: u32) -> Option<&str> {
    item.get("singleValueExtendedProperties")?
        .as_array()?
        .iter()
        .find(|p| {
            p.get("id")
                .and_then(Value::as_str)
                .and_then(property_tag)
                .is_some_and(|t| t == tag)
        })?
        .get("value")?
        .as_str()
}

/// The tag of an extended property's id: `Integer 0x0E08` and `Integer 0xe08` are both 0x0E08.
fn property_tag(id: &str) -> Option<u32> {
    let hex = id.split_whitespace().nth(1)?;
    let hex = hex.strip_prefix("0x").or_else(|| hex.strip_prefix("0X"))?;
    u32::from_str_radix(hex, 16).ok()
}

fn header(out: &mut String, name: &str, value: &str) {
    // A line break in a value would start a header of the sender's choosing.
    let value: String = value
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect();
    out.push_str(name);
    out.push_str(": ");
    out.push_str(&value);
    out.push_str("\r\n");
}

/// Graph's recipients as an address list.
fn recipients(list: &[Value]) -> String {
    list.iter()
        .filter_map(|r| {
            let email = r.get("emailAddress")?;
            let address = email.get("address").and_then(Value::as_str)?;
            if address.is_empty() {
                return None;
            }
            Some(
                match email
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty() && *n != address)
                {
                    Some(name) => format!("{} <{address}>", display_name(name)),
                    None => address.to_owned(),
                },
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A display name as a phrase: quoted where it is ASCII, encoded (RFC 2047) where it is not.
fn display_name(name: &str) -> String {
    if name.is_ascii() {
        let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        encoded(name)
    }
}

/// `text` as RFC 2047 encoded words where it is not ASCII, and as itself where it is.
fn encoded(text: &str) -> String {
    if text.is_ascii() {
        return text.to_owned();
    }
    // Encoded words of at most 45 bytes of text each, so none passes 75 characters, split on
    // character boundaries so no word ends in half a character.
    let mut words = Vec::new();
    let mut start = 0;
    let mut end = 0;
    for (at, c) in text.char_indices() {
        if at + c.len_utf8() - start > 45 {
            words.push(&text[start..end]);
            start = end;
        }
        end = at + c.len_utf8();
    }
    words.push(&text[start..end]);
    words
        .into_iter()
        .map(|w| {
            format!(
                "=?UTF-8?B?{}?=",
                base64::engine::general_purpose::STANDARD.encode(w)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The backend slot of an engine whose account reads through Graph.
///
/// The engine's operations go to its [`Reader`] instead (see
/// `AccountEngine::with_graph_reader`), so nothing ever drives this: it answers every operation
/// by saying so, which is the honest answer if something ever does.
#[derive(Debug)]
pub struct OverHttp {
    caps: AccountCaps,
}

impl OverHttp {
    pub fn new(caps: AccountCaps) -> Self {
        Self { caps }
    }
}

impl mail_proto::Backend for OverHttp {
    fn begin(&mut self, _op: ProtoOp) -> mail_proto::Progress<ProtoOutcome> {
        mail_proto::Progress::Failed(mail_proto::ProtoError::Unsupported(
            "an account read through Microsoft Graph has no session to drive".to_owned(),
        ))
    }

    fn feed(&mut self, _ready: mail_proto::IoReady) -> mail_proto::Progress<ProtoOutcome> {
        self.begin(ProtoOp::FetchCaps)
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delta_entry_becomes_a_header_block_a_parser_reads() {
        let item = json!({
            "id": "AAMk1",
            "internetMessageId": "<m1@example.test>",
            "sentDateTime": "2026-09-20T08:30:00Z",
            "isRead": true,
            "flag": { "flagStatus": "flagged" },
            "subject": "Quarterly numbers",
            "from": { "emailAddress": { "name": "Ada \"the\" Analyst", "address": "ada@example.test" } },
            "toRecipients": [
                { "emailAddress": { "name": "Me", "address": "me@example.test" } },
                { "emailAddress": { "address": "bob@example.test" } }
            ],
            "ccRecipients": [],
            "singleValueExtendedProperties": [
                { "id": "Integer 0xe08", "value": "2048" },
                { "id": "String 0x1042", "value": "<parent@example.test>" },
                { "id": "String 0x1039", "value": "<root@example.test> <parent@example.test>" }
            ]
        });
        let described = describe(&item);
        assert_eq!(described.read, ReadState::Read);
        assert_eq!(described.star, Star::Starred);
        assert_eq!(described.size, 2048);
        let parsed = mail_mime::parse(&described.headers).unwrap();
        assert_eq!(parsed.subject, "Quarterly numbers");
        assert_eq!(parsed.rfc_message_id.as_deref(), Some("m1@example.test"));
        assert_eq!(parsed.in_reply_to.as_deref(), Some("parent@example.test"));
        assert_eq!(parsed.references.len(), 2);
        assert_eq!(parsed.from.unwrap().email, "ada@example.test");
        assert_eq!(parsed.to.len(), 2);
        assert!(parsed.date.is_some());
    }

    #[test]
    fn a_subject_that_is_not_ascii_survives_the_header() {
        let item = json!({
            "id": "x",
            "subject": "會議記錄：第三季的預算與人事安排，請在週五前回覆",
            "from": { "emailAddress": { "name": "陳小姐", "address": "chen@example.test" } },
        });
        let parsed = mail_mime::parse(&describe(&item).headers).unwrap();
        assert_eq!(
            parsed.subject,
            "會議記錄：第三季的預算與人事安排，請在週五前回覆"
        );
        assert_eq!(parsed.from.unwrap().name.as_deref(), Some("陳小姐"));
    }

    #[test]
    fn a_line_break_in_a_property_cannot_start_a_header() {
        let item = json!({ "id": "x", "subject": "hi\r\nBcc: eve@example.test" });
        let text = String::from_utf8(describe(&item).headers).unwrap();
        assert!(!text.contains("\r\nBcc:"), "{text}");
    }

    #[test]
    fn a_message_with_no_size_is_fetched_last() {
        assert_eq!(describe(&json!({ "id": "x" })).size, u64::MAX);
    }

    #[test]
    fn property_ids_are_read_whatever_their_spelling() {
        assert_eq!(property_tag("Integer 0x0E08"), Some(0x0E08));
        assert_eq!(property_tag("Integer 0xe08"), Some(0x0E08));
        assert_eq!(property_tag("String 0x1042"), Some(0x1042));
        assert_eq!(property_tag("nonsense"), None);
    }

    #[test]
    fn throttling_waits_as_long_as_graph_says() {
        let RuntimeError::Graph { retry, .. } = refused(429, "", Some(Duration::from_secs(17)))
        else {
            panic!()
        };
        assert_eq!(retry, Retry::After(Duration::from_secs(17)));
        let RuntimeError::Graph { retry, .. } = refused(401, "", None) else {
            panic!()
        };
        assert_eq!(retry, Retry::NeedsReauth);
    }
}
