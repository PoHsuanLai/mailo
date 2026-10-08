//! CardDAV's HTTP through accountd's relay (porter, step E7): the one adapter between hyper's
//! client and the stream `Accounts::open_authenticated` returns.
//!
//! What the relay takes is plain HTTP/1.1 on the stream it hands over, and it does the rest
//! (`porter_proxy::http1`): it dials the endpoint's origin over TLS, drops any `Authorization` the
//! app wrote and adds its own, refuses a `Host` or an absolute-form target that is not the
//! endpoint's, and passes bodies and responses through. So a request here is a request line in
//! origin-form, a few headers and a body, and never a credential; reqwest cannot be pointed at a
//! stream it is given (it has no connector for one), which is why hyper's client connection is
//! used, with the stream under it and nothing else.
//!
//! **One stream for each request.** The relay does not frame responses, so a connection held
//! between requests would be found closed whenever the server closed it while idle (several
//! CardDAV servers close after every reply, as the test server does), and a retry after that is
//! not safe for a `PUT`. A stream costs one call to accountd and one TLS handshake by the relay; a
//! sync is a handful of requests.

use super::{CardDavFailure, Raw, TIMEOUT, unreachable};
use crate::link::{Accountd, LinkError};
use crate::transport::relayed_io;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::rt::TokioIo;
use mail_domain::Retry;
use porter_core::{GrantId, ServiceEndpoint};
use reqwest::Method;
use std::sync::Arc;
use url::Url;

/// The relay of one account's CardDAV endpoint.
#[derive(Debug, Clone)]
pub(super) struct Relay {
    link: Arc<dyn Accountd>,
    grant: GrantId,
    endpoint: ServiceEndpoint,
}

impl Relay {
    pub(super) fn new(link: Arc<dyn Accountd>, grant: GrantId, endpoint: ServiceEndpoint) -> Self {
        Self {
            link,
            grant,
            endpoint,
        }
    }

    pub(super) fn endpoint(&self) -> &ServiceEndpoint {
        &self.endpoint
    }

    /// One request over a stream of its own, within [`TIMEOUT`].
    pub(super) async fn exchange(
        &self,
        method: &Method,
        url: &Url,
        headers: &[(&'static str, String)],
        body: &str,
    ) -> Result<Raw, CardDavFailure> {
        tokio::time::timeout(TIMEOUT, self.once(method, url, headers, body))
            .await
            .unwrap_or_else(|_| {
                Err(CardDavFailure::Unreachable(
                    "the relay did not answer in time".to_owned(),
                ))
            })
    }

    async fn once(
        &self,
        method: &Method,
        url: &Url,
        headers: &[(&'static str, String)],
        body: &str,
    ) -> Result<Raw, CardDavFailure> {
        let stream = self
            .link
            .open_stream(&self.grant, &self.endpoint)
            .await
            .map_err(refused)?;
        let io = relayed_io(stream).map_err(|e| CardDavFailure::Unreachable(e.to_string()))?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(io))
                .await
                .map_err(|e| unreachable(&e))?;
        // Drives the connection while the request is in flight; the stream goes with it.
        let driver = tokio::spawn(connection);

        // Origin-form, as the relay wants it. It adds `Host` itself, from the endpoint.
        let target = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let mut request = Request::builder().method(method.clone()).uri(target);
        for (name, value) in headers {
            request = request.header(*name, value.as_str());
        }
        let request = request
            .body(Full::new(Bytes::from(body.to_owned())))
            .map_err(|e| CardDavFailure::Malformed(e.to_string()))?;

        let result = async {
            let response = sender
                .send_request(request)
                .await
                .map_err(|e| unreachable(&e))?;
            let (parts, body) = response.into_parts();
            let bytes = body
                .collect()
                .await
                .map_err(|e| unreachable(&e))?
                .to_bytes();
            let header = |name: hyper::header::HeaderName| {
                parts
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            };
            Ok(Raw {
                status: parts.status,
                location: header(hyper::header::LOCATION),
                etag: header(hyper::header::ETAG),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            })
        }
        .await;
        driver.abort();
        result
    }
}

/// accountd's answer to opening the relay, as a CardDAV failure: a refusal only the person can
/// answer (the account needs signing in, the grant is gone) is `Unauthorized`, which routes as
/// the sign-in being refused does; a daemon that is not there now is a failure to reach.
fn refused(error: LinkError) -> CardDavFailure {
    match error.retry() {
        Retry::NeedsReauth => CardDavFailure::Unauthorized,
        Retry::Now | Retry::After(_) => CardDavFailure::Unreachable(error.to_string()),
        Retry::Fatal(why) => CardDavFailure::Malformed(why),
    }
}
