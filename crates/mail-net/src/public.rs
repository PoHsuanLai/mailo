//! Fetches a message or a sender's domain chose the address of, kept on the public internet.
//!
//! A remote image, a BIMI logo, a WKD key and a one-click unsubscribe are each fetched from where
//! someone else said: the message's HTML, the sender's DNS, a header.
//! Pointed at `127.0.0.1`, a router's `192.168.1.1` or a cloud's `169.254.169.254`, such a fetch
//! is the reader's machine making a request inside their own network on a stranger's behalf, and
//! telling the stranger, by whether it loaded, what answered. So these clients reach public
//! addresses only: [`guarded`] gives a client a resolver that drops every other address a name
//! resolves to, and [`url_is_public`] refuses a URL (or a redirect) that names one directly, which
//! never reaches a resolver at all.
//!
//! The servers a person configured (IMAP, CardDAV, JMAP) are not fetched through here: one on
//! their own network is theirs to choose. Nor is account discovery, which starts from the
//! person's own address: a self-hosted server at `192.168.1.10` or `mail.home.arpa` is the one
//! they mean to find.
//!
//! These clients use no proxy. A proxy resolves the name itself, so behind one the resolver here
//! would never be asked and the guard would be no guard.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

/// Whether `ip` is an address on the public internet: not loopback, private, link-local,
/// shared (carrier-grade NAT), unspecified, broadcast, multicast or documentation, and not an
/// IPv4 address of one of those carried in IPv6.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => v4_is_public(ip),
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(v4) => v4_is_public(v4),
            None => v6_is_public(ip),
        },
    }
}

fn v4_is_public(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 0.0.0.0/8, "this network", and 100.64.0.0/10, shared address space (RFC 6598).
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24, protocol assignments, and 198.18.0.0/15, benchmarking.
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        || (a == 198 && (18..20).contains(&b))
        // 240.0.0.0/4, reserved.
        || a >= 240)
}

fn v6_is_public(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        // fec0::/10, the deprecated site-local range, and 2001:db8::/32, documentation.
        || (first & 0xffc0) == 0xfec0
        || (first == 0x2001 && ip.segments()[1] == 0x0db8)
        // ::/96, IPv4-compatible, the old way of carrying a v4 address.
        || ip.segments()[..6].iter().all(|s| *s == 0)
        // 64:ff9b:1::/48, local-use NAT64 (RFC 8215), which is not globally reachable.
        || ip.segments()[..3] == [0x64, 0xff9b, 1])
        && carried_v4(ip).is_none_or(v4_is_public)
}

/// The IPv4 address a translated or tunnelled IPv6 address reaches: the last 32 bits of the
/// well-known NAT64 prefix `64:ff9b::/96` (RFC 6052), and the 32 bits after `2002` in 6to4's
/// `2002::/16` (RFC 3056). A router on the way delivers to that address, so it is what is public
/// or not.
fn carried_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = ip.segments();
    let v4 = |high: u16, low: u16| Ipv4Addr::from((u32::from(high) << 16) | u32::from(low));
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        Some(v4(s[6], s[7]))
    } else if s[0] == 0x2002 {
        Some(v4(s[1], s[2]))
    } else {
        None
    }
}

/// Whether a host name is one that only ever names something on this machine or its network.
fn names_the_local_network(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || [".localhost", ".local", ".internal", ".home.arpa"]
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

/// Whether `url` names a public host: a name not of the local network, or a public address.
/// A name is checked again, once resolved, by [`guarded`]'s resolver.
pub fn url_is_public(url: &reqwest::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => !names_the_local_network(name),
        Some(url::Host::Ipv4(ip)) => is_public(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_public(IpAddr::V6(ip)),
        None => false,
    }
}

/// `builder` with a resolver that keeps only the public addresses a name resolves to, and fails
/// a name that has none, and with no proxy, which would resolve the name past that resolver.
pub fn guarded(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder.dns_resolver(Arc::new(PublicOnly)).no_proxy()
}

/// The system's resolver, with every address that is not public dropped.
#[derive(Debug)]
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            if names_the_local_network(&host) {
                return Err(not_public(&host));
            }
            let all = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let public: Vec<SocketAddr> = all.filter(|at| is_public(at.ip())).collect();
            if public.is_empty() {
                return Err(not_public(&host));
            }
            let addrs: Addrs = Box::new(public.into_iter());
            Ok(addrs)
        })
    }
}

fn not_public(host: &str) -> Box<dyn std::error::Error + Send + Sync> {
    format!("{host} is not on the public internet").into()
}

#[cfg(test)]
mod tests {
    use super::{PublicOnly, is_public, url_is_public};
    use reqwest::dns::Resolve;
    use std::net::IpAddr;

    #[test]
    fn only_public_addresses_are_public() {
        let cases = [
            ("8.8.8.8", true),
            ("1.1.1.1", true),
            ("2606:4700:4700::1111", true),
            ("127.0.0.1", false),
            ("10.0.0.1", false),
            ("172.16.5.4", false),
            ("192.168.1.1", false),
            ("169.254.169.254", false),
            ("100.64.0.1", false),
            ("0.0.0.0", false),
            ("0.1.2.3", false),
            ("255.255.255.255", false),
            ("224.0.0.1", false),
            ("192.0.2.1", false),
            ("198.18.0.1", false),
            ("240.0.0.1", false),
            ("::1", false),
            ("::", false),
            ("fe80::1", false),
            ("fc00::1", false),
            ("fd12:3456::1", false),
            ("fec0::1", false),
            ("ff02::1", false),
            ("2001:db8::1", false),
            ("::ffff:127.0.0.1", false),
            ("::ffff:169.254.169.254", false),
            ("::ffff:8.8.8.8", true),
            ("::127.0.0.1", false),
            // NAT64 and 6to4 reach the IPv4 address inside them.
            ("64:ff9b::7f00:1", false),
            ("64:ff9b::a9fe:a9fe", false),
            ("64:ff9b::808:808", true),
            ("64:ff9b:1::808:808", false),
            ("2002:c0a8:101::1", false),
            ("2002:7f00:1::1", false),
            ("2002:808:808::1", true),
        ];
        for (ip, public) in cases {
            let parsed: IpAddr = ip.parse().unwrap();
            assert_eq!(is_public(parsed), public, "{ip}");
        }
    }

    #[test]
    fn a_url_naming_the_local_network_is_not_public() {
        let cases = [
            ("https://example.com/a.png", true),
            ("https://8.8.8.8/", true),
            ("https://127.0.0.1/", false),
            ("http://169.254.169.254/latest/meta-data/", false),
            ("https://[::1]:8443/", false),
            ("https://[::ffff:10.0.0.1]/", false),
            ("https://localhost/", false),
            ("https://LOCALHOST./", false),
            ("https://printer.local/", false),
            ("https://router.home.arpa/", false),
            ("https://metadata.google.internal/", false),
            ("https://a.localhost/", false),
        ];
        for (url, public) in cases {
            let parsed = reqwest::Url::parse(url).unwrap();
            assert_eq!(url_is_public(&parsed), public, "{url}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_only_to_this_machine_is_not_reached() {
        for name in ["localhost", "printer.local"] {
            let resolved = PublicOnly.resolve(name.parse().unwrap()).await;
            assert!(resolved.is_err(), "{name}");
        }
    }
}
