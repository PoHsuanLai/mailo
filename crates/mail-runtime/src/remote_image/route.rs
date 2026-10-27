//! Which way a remote image is fetched: straight from the server that holds it, or through
//! something that hides where the reader is.
//!
//! Fetched straight, an image tells its host the reader's address, and so roughly where they are
//! and on which network, every time a message is opened. Three ways round that, none tied to any
//! one host:
//!
//! - **A Camo relay** ([`Route::Camo`]): the URL format of GitHub's camo, which go-camo and
//!   imgproxy also serve. The image's URL is written in hex after an HMAC-SHA1 of it under the
//!   key the relay was set up with, so the relay fetches only what someone holding the key asked
//!   for. Anyone can run go-camo in one container.
//! - **A token relay** ([`Route::Token`]): `GET {base}/i?u=<url>` with the token as a bearer
//!   credential. `relay/image/worker.js` in this repository is one, for Cloudflare Workers.
//! - **A proxy** ([`Route::Proxy`]): an HTTP or SOCKS5 proxy the person names (Tor, a VPN's), which
//!   carries the request; the image's host sees the proxy's address.
//!
//! Through a relay or a proxy the relay is what reaches the image's host, so the resolver that
//! keeps a fetch on the public internet (`public.rs`) is not asked: a relay the person set up is
//! theirs to choose, like their mail server. The image's own URL is still refused when it names
//! a private address directly, so a message cannot point the relay inside the reader's network
//! by a URL alone.

use hmac::{Hmac, Mac};
use sha1::Sha1;

/// Which way a remote image is fetched.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum Route {
    /// Straight from the image's host, through the public-only resolver.
    #[default]
    Direct,
    /// Through a Camo relay at `base`, with URLs signed under `key`.
    Camo { base: reqwest::Url, key: String },
    /// Through a token relay at `base`, which takes `token` as a bearer credential.
    Token { base: reqwest::Url, token: String },
    /// Through the proxy at `url` (`http://`, `https://`, `socks5://` or `socks5h://`).
    Proxy { url: String },
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key and the token are secrets; a log line names only the way.
        match self {
            Route::Direct => f.write_str("Direct"),
            Route::Camo { base, .. } => write!(f, "Camo({base})"),
            Route::Token { base, .. } => write!(f, "Token({base})"),
            Route::Proxy { .. } => f.write_str("Proxy"),
        }
    }
}

impl Route {
    /// The URL that is asked, for the image at `image`: the image's own for [`Route::Direct`]
    /// and [`Route::Proxy`], the relay's for the others.
    pub fn asked(&self, image: &reqwest::Url) -> reqwest::Url {
        match self {
            Route::Direct | Route::Proxy { .. } => image.clone(),
            Route::Camo { base, key } => {
                let digest = hex(&sign(key.as_bytes(), image.as_str().as_bytes()));
                let mut at = base.clone();
                let path = format!(
                    "{}/{digest}/{}",
                    base.path().trim_end_matches('/'),
                    hex(image.as_str().as_bytes())
                );
                at.set_path(&path);
                at.set_query(None);
                at
            }
            Route::Token { base, .. } => {
                let mut at = base.clone();
                let path = format!("{}/i", base.path().trim_end_matches('/'));
                at.set_path(&path);
                at.query_pairs_mut()
                    .clear()
                    .append_pair("u", image.as_str());
                at
            }
        }
    }

    /// The bearer credential a request carries, for a token relay.
    pub fn bearer(&self) -> Option<&str> {
        match self {
            Route::Token { token, .. } => Some(token),
            _ => None,
        }
    }
}

/// HMAC-SHA1 of `message` under `key`, as Camo signs.
fn sign(key: &[u8], message: &[u8]) -> Vec<u8> {
    // HMAC takes a key of any length; `new_from_slice` refuses none.
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

/// `bytes` as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> reqwest::Url {
        reqwest::Url::parse(text).unwrap()
    }

    #[test]
    fn each_route_asks_its_own_url() {
        let image = url("https://images.example.test/a.png?w=2");
        let cases = [
            (Route::Direct, "https://images.example.test/a.png?w=2"),
            (
                Route::Proxy {
                    url: "socks5h://127.0.0.1:9050".to_owned(),
                },
                "https://images.example.test/a.png?w=2",
            ),
            (
                Route::Camo {
                    base: url("https://camo.example.test/"),
                    key: "0x24FEEDFACEDEADBEEFCAFE".to_owned(),
                },
                "https://camo.example.test/a55db9b2117ed695e3c80a437f3a3114e5b5aca9/68747470733a2f2f696d616765732e6578616d706c652e746573742f612e706e673f773d32",
            ),
            (
                Route::Token {
                    base: url("https://relay.example.test/mail"),
                    token: "t".to_owned(),
                },
                "https://relay.example.test/mail/i?u=https%3A%2F%2Fimages.example.test%2Fa.png%3Fw%3D2",
            ),
        ];
        for (route, want) in cases {
            assert_eq!(route.asked(&image).as_str(), want, "{route:?}");
        }
    }

    #[test]
    fn camo_signs_as_rfc_2202_says_hmac_sha1_does() {
        // RFC 2202, test case 2.
        assert_eq!(
            hex(&sign(b"Jefe", b"what do ya want for nothing?")),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
    }

    #[test]
    fn a_route_never_writes_its_secret_in_a_log() {
        let route = Route::Token {
            base: url("https://relay.example.test/"),
            token: "very-secret".to_owned(),
        };
        assert!(!format!("{route:?}").contains("very-secret"));
        assert_eq!(route.bearer(), Some("very-secret"));
    }
}
