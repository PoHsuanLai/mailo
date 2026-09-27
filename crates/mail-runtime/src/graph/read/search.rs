//! Searching a mailbox through Microsoft Graph: `GET …/messages` with `$search` (KQL) or
//! `$filter`, never both and never `$orderby` with `$search`, as Graph requires of messages.
//!
//! One page, of at most the number asked for. Each message comes with the properties a delta
//! asks for, so its header is written from the answer as a delta entry's is ([`describe`]) and
//! nothing more is requested: no body, nothing it links to.

use super::{Answer, EXPAND, INBOX, Reader, SELECT, describe, well_known};
use crate::RuntimeError;
use crate::graph::segment;
use mail_domain::{ReadState, RemoteRef, Star};
use mail_proto::search::graph::{GraphPlace, GraphPlan, GraphQuery};
use serde_json::Value;

/// One message a search found, as the store will keep it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphHit {
    /// The folder it is in, by this client's path for it, and its id.
    pub remote: RemoteRef,
    /// Its header, written from the search's answer.
    pub headers: Vec<u8>,
    pub read: ReadState,
    pub star: Star,
}

/// What one search found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphFound {
    /// Newest first where Graph orders them so; in its own relevance order under `$search`.
    pub hits: Vec<GraphHit>,
    /// Whether Graph named a next page: more matched than were taken.
    pub more: bool,
}

impl Reader {
    /// Ask Graph for at most `top` messages matching `plan`, with the access token `token`.
    pub async fn search(
        &mut self,
        plan: &GraphPlan,
        token: &str,
        top: usize,
    ) -> Result<GraphFound, RuntimeError> {
        let base = match &plan.place {
            GraphPlace::Everywhere => format!("{}/messages", self.me),
            GraphPlace::Role(role) => {
                format!("{}/mailFolders/{}/messages", self.me, well_known(*role))
            }
            GraphPlace::Folder(path) => {
                let id = self.folder_id(path, token).await?;
                format!("{}/mailFolders/{}/messages", self.me, segment(&id))
            }
        };
        let mut url = url::Url::parse(&base)
            .map_err(|e| RuntimeError::Connect(format!("Microsoft Graph: {e}")))?;
        {
            let mut query = url.query_pairs_mut();
            match &plan.query {
                GraphQuery::All => {}
                // The KQL goes inside one quoted string; a quotation mark inside it is escaped.
                GraphQuery::Search(kql) => {
                    query.append_pair("$search", &format!("\"{}\"", kql.replace('"', "\\\"")));
                }
                GraphQuery::Filter(odata) => {
                    query.append_pair("$filter", odata);
                }
            }
            query
                .append_pair("$top", &top.to_string())
                .append_pair("$select", SELECT)
                .append_pair("$expand", EXPAND);
        }
        let page: Value = match self
            .call(reqwest::Method::GET, url.as_str(), token, None)
            .await?
        {
            Answer::Done(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                RuntimeError::Proto(mail_proto::ProtoError::Malformed(format!(
                    "Microsoft Graph's search answer: {e}"
                )))
            })?,
            // A folder that has gone since it was listed holds nothing to find.
            Answer::Gone => return Ok(GraphFound::default()),
        };
        let items: Vec<Value> = page
            .get("value")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut hits = Vec::new();
        let mut listed = false;
        for item in items.iter().take(top) {
            let (Some(id), Some(folder)) = (
                item.get("id").and_then(Value::as_str),
                item.get("parentFolderId").and_then(Value::as_str),
            ) else {
                continue;
            };
            // A folder this client does not know of — hidden, or made since the last listing —
            // has no path to keep the message under; the message is left on the server.
            let Some(path) = self.path_of(folder, token, &mut listed).await? else {
                continue;
            };
            let described = describe(item);
            hits.push(GraphHit {
                remote: RemoteRef::Graph {
                    mailbox: path,
                    id: id.to_owned(),
                },
                headers: described.headers,
                read: described.read,
                star: described.star,
            });
        }
        Ok(GraphFound {
            hits,
            more: page.get("@odata.nextLink").is_some() || items.len() > top,
        })
    }

    /// This client's path for the folder Graph calls `id`, listing the folders if it is not
    /// known yet and they have not been listed for this search.
    async fn path_of(
        &mut self,
        id: &str,
        token: &str,
        listed: &mut bool,
    ) -> Result<Option<String>, RuntimeError> {
        let known = |folders: &std::collections::HashMap<String, String>| {
            folders
                .iter()
                .find(|(_, folder)| folder.as_str() == id)
                .map(|(path, _)| path.clone())
        };
        if let Some(path) = known(&self.folders) {
            return Ok(Some(path));
        }
        if *listed {
            return Ok(None);
        }
        *listed = true;
        self.list(token).await?;
        Ok(known(&self.folders).or_else(|| (id == "inbox").then(|| INBOX.to_owned())))
    }
}
