//! The HTTP client the token endpoints, Graph and JMAP are reached with.
//!
//! One place, because a request with no timeout is a setup command that hangs forever and a sync
//! pass that never returns. (Discovery's client is [`crate::lookup`]'s: https only, and shorter.)

use crate::NetError;
use crate::error::Failure;
use porter_http_reqwest::ReqwestHttp;
use std::time::Duration;

/// A token endpoint that never answers must not hang a setup command or a sync pass forever.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// The shared client: built once, since reading the system's roots is not cheap and a pass
/// asks for it every few seconds. A clone shares its connections.
pub fn http_client() -> Result<reqwest::Client, NetError> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = crate::roots::trusting_system(reqwest::Client::builder())
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| NetError::Connect(Failure::new("cannot build an HTTP client", e)))?;
    Ok(CLIENT.get_or_init(|| client).clone())
}

/// `porter_http`'s seam over porter's own client, which is what porter-oauth sends its exchanges
/// over. Built once, like [`http_client`], and a clone shares its connections.
pub fn oauth_http() -> ReqwestHttp {
    static HTTP: std::sync::OnceLock<ReqwestHttp> = std::sync::OnceLock::new();
    HTTP.get_or_init(ReqwestHttp::new).clone()
}

/// The most a JSON answer (Graph, JMAP, CardDAV) is read to. The largest real ones are a page of
/// a mailbox's changes, a few megabytes.
pub const MAX_DOCUMENT: usize = 64 * 1024 * 1024;

/// The most a refusal's body is read to: it is only read for its words.
pub const MAX_REFUSAL: usize = 64 * 1024;

/// Why [`body_up_to`] has no body.
#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    /// The server said, or sent, more than the limit.
    #[error("the answer is past the {0} bytes it may be")]
    TooLarge(usize),
    /// The body stopped arriving.
    #[error(transparent)]
    Read(#[from] reqwest::Error),
}

/// `response`'s body, read no further than `max` bytes: one that says or grows longer is
/// [`BodyError::TooLarge`], so a server cannot fill the client's memory by not stopping.
pub async fn body_up_to(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, BodyError> {
    if response
        .content_length()
        .is_some_and(|len| len > max as u64)
    {
        return Err(BodyError::TooLarge(max));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > max {
            return Err(BodyError::TooLarge(max));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// [`body_up_to`] as text, with any byte that is not UTF-8 replaced.
pub async fn text_up_to(response: reqwest::Response, max: usize) -> Result<String, BodyError> {
    let body = body_up_to(response, max).await?;
    Ok(String::from_utf8(body)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::{BodyError, body_up_to};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One answer from a server on loopback that says `head` and then sends `body`.
    async fn answer(head: &'static str, body: Vec<u8>) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            // The request is not looked at; reading it lets the client go on to the answer.
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&body).await;
        });
        reqwest::get(format!("http://{at}/")).await.unwrap()
    }

    #[tokio::test]
    async fn a_body_is_read_up_to_the_limit_and_no_further() {
        let closed = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n";
        let fits = answer(closed, vec![b'x'; 100]).await;
        assert_eq!(body_up_to(fits, 100).await.unwrap().len(), 100);

        let grows = answer(closed, vec![b'x'; 101]).await;
        assert!(matches!(
            body_up_to(grows, 100).await,
            Err(BodyError::TooLarge(100))
        ));

        let says = "HTTP/1.1 200 OK\r\nContent-Length: 1000000000\r\n\r\n";
        let claimed = answer(says, Vec::new()).await;
        assert!(matches!(
            body_up_to(claimed, 100).await,
            Err(BodyError::TooLarge(100))
        ));
    }
}
