//! The loopback listener that catches an OAuth redirect.
//!
//! An installed application cannot keep a client secret, so the authorization code comes back
//! to `http://127.0.0.1:<port>` and PKCE proves the exchange belongs to us. That makes this a
//! listener anything on the machine can reach, which is why it validates before it trusts.

use crate::RuntimeError;
use crate::oauth::Pending;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Give up if no browser arrives. The user may have closed the tab or never signed in.
const WAIT: Duration = Duration::from_secs(300);

/// Refuse a request line longer than this. A loopback listener is reachable by anything on the
/// machine, and an unbounded read is a free denial of service.
const MAX_REQUEST: usize = 8 * 1024;

/// A listener bound to a random loopback port.
#[derive(Debug)]
pub struct Loopback {
    listener: TcpListener,
    redirect: String,
}

impl Loopback {
    /// Bind to `127.0.0.1` on a port the OS chooses.
    ///
    /// Explicitly not `0.0.0.0`: binding the wildcard would accept an authorization code from
    /// anywhere on the network.
    pub async fn bind() -> Result<Self, RuntimeError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| RuntimeError::Connect(format!("loopback: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| RuntimeError::Connect(e.to_string()))?
            .port();
        Ok(Self {
            listener,
            redirect: format!("http://127.0.0.1:{port}"),
        })
    }

    /// The redirect URI to register with the authorization request.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect
    }

    /// Wait for the redirect and return the authorization code.
    ///
    /// Rejects any request whose `state` does not match `pending`, and keeps waiting: a
    /// mismatched state is another party's code being offered to us, and treating it as the
    /// answer would be the exact attack the parameter prevents.
    pub async fn wait_for_code(&self, pending: &Pending) -> Result<String, RuntimeError> {
        tokio::time::timeout(WAIT, self.accept_loop(pending))
            .await
            .map_err(|_| RuntimeError::Secrets("timed out waiting for the browser".to_owned()))?
    }

    async fn accept_loop(&self, pending: &Pending) -> Result<String, RuntimeError> {
        loop {
            let (mut sock, _) = self
                .listener
                .accept()
                .await
                .map_err(|e| RuntimeError::Io(e.to_string()))?;

            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            // Read only to the end of the request line and headers; we never need a body.
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < MAX_REQUEST {
                match sock.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }

            let request = String::from_utf8_lossy(&buf);
            let target = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            let params = query_pairs(target);

            let code = params.iter().find(|(k, _)| k == "code").map(|(_, v)| v);
            let state = params.iter().find(|(k, _)| k == "state").map(|(_, v)| v);
            let denied = params.iter().find(|(k, _)| k == "error").map(|(_, v)| v);

            match (code, state) {
                (Some(code), Some(state)) if pending.accepts(state) => {
                    respond(&mut sock, "Signed in. You can close this tab.").await;
                    return Ok(code.clone());
                }
                _ if denied.is_some() => {
                    respond(&mut sock, "Authorization was declined.").await;
                    return Err(RuntimeError::Secrets(format!(
                        "authorization declined: {}",
                        denied.map(String::as_str).unwrap_or("unknown")
                    )));
                }
                _ => {
                    // Anything else on this port: a probe, a favicon request, or a redirect
                    // with the wrong state. Say nothing useful and keep waiting for the real one.
                    respond(&mut sock, "Not what this port is for.").await;
                }
            }
        }
    }
}

/// Decode a query string into pairs. Percent-decoding by hand to avoid a dependency for this.
fn query_pairs(target: &str) -> Vec<(String, String)> {
    let Some((_, query)) = target.split_once('?') else {
        return Vec::new();
    };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (percent_decode(k), percent_decode(v)))
        .collect()
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    // Malformed escape: keep the '%' literally rather than dropping input.
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn respond(sock: &mut tokio::net::TcpStream, message: &str) {
    // Deliberately plain text and self-contained: no remote resources, nothing reflected from
    // the request, so a crafted URL cannot put content in the user's browser.
    let body = format!("<!doctype html><meta charset=utf-8><p>{message}");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(response.as_bytes()).await;
    let _ = sock.flush().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_pairs_decodes_escapes() {
        let pairs = query_pairs("/?code=4%2F0Aa&state=x%20y&scope=a+b");
        assert_eq!(pairs[0], ("code".into(), "4/0Aa".into()));
        assert_eq!(pairs[1], ("state".into(), "x y".into()));
        assert_eq!(pairs[2], ("scope".into(), "a b".into()));
    }

    #[test]
    fn a_malformed_escape_is_kept_rather_than_dropped() {
        // Silently dropping input is how a truncated code turns into a confusing failure far
        // from its cause.
        assert_eq!(percent_decode("a%zz"), "a%zz");
        assert_eq!(percent_decode("trailing%"), "trailing%");
    }

    #[test]
    fn a_target_without_a_query_yields_nothing() {
        assert!(query_pairs("/favicon.ico").is_empty());
        assert!(query_pairs("").is_empty());
    }

    #[tokio::test]
    async fn binds_to_loopback_only() {
        // Binding the wildcard would accept an authorization code from anywhere on the network.
        let lb = Loopback::bind().await.unwrap();
        assert!(
            lb.redirect_uri().starts_with("http://127.0.0.1:"),
            "{}",
            lb.redirect_uri()
        );
        assert_eq!(
            lb.listener.local_addr().unwrap().ip().to_string(),
            "127.0.0.1"
        );
    }

    #[tokio::test]
    async fn a_redirect_with_the_wrong_state_does_not_end_the_wait() {
        // The attack this defends against: another party's code delivered to our listener.
        let lb = Loopback::bind().await.unwrap();
        let auth = crate::oauth::begin(
            mail_domain::OAuthIssuer::Google,
            "client.apps.googleusercontent.com",
            None,
            &["https://mail.google.com/".to_owned()],
            lb.redirect_uri(),
        )
        .unwrap();
        let port = lb.listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            sock.write_all(b"GET /?code=stolen&state=wrong HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut sink = Vec::new();
            let _ = sock.read_to_end(&mut sink).await;
        });

        // The listener must still be waiting, not have returned the stolen code.
        let outcome =
            tokio::time::timeout(Duration::from_millis(400), lb.accept_loop(&auth.pending)).await;
        assert!(
            outcome.is_err(),
            "a mismatched state must not satisfy the wait"
        );
    }
}
