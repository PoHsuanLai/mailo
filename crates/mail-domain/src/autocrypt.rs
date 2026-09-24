//! Autocrypt Level 1 peer state, and the rules that update it.
//!
//! Autocrypt spreads keys by putting the sender's key in a header of every message it sends.
//! A receiving client keeps, per correspondent, what the spec calls the peer state: when mail
//! from them was last seen, when an Autocrypt header was last seen and which key it carried,
//! and — from `Autocrypt-Gossip` headers inside encrypted mail — what other people's clients
//! say their key is. The rules below are the spec's (Autocrypt Level 1, "Updating Autocrypt Peer
//! State" and "Updating Autocrypt Peer State from Key Gossip"); the keys themselves are
//! [`crate::PgpKey`] rows the peer state points at by fingerprint.

use crate::pgp::Fingerprint;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Whether a correspondent asked for encrypted replies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferEncrypt {
    /// No `prefer-encrypt` attribute, or one with a value the spec does not define.
    #[default]
    NoPreference,
    /// `prefer-encrypt=mutual`: encrypt by default when both sides said so.
    Mutual,
}

/// What this client knows about one correspondent's Autocrypt key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutocryptPeer {
    /// The address, lower-cased.
    pub address: String,
    /// When mail from them was last seen. `None` for a peer known only by gossip.
    pub last_seen: Option<DateTime<Utc>>,
    /// When their last Autocrypt header was seen.
    pub autocrypt_timestamp: Option<DateTime<Utc>>,
    /// The key that header carried.
    pub key: Option<Fingerprint>,
    pub prefer_encrypt: PreferEncrypt,
    /// When a third party last gossiped a key for them.
    pub gossip_timestamp: Option<DateTime<Utc>>,
    /// The key that gossip carried.
    pub gossip_key: Option<Fingerprint>,
}

/// One message from `address`, as the peer-state rules see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    /// The sender's address.
    pub address: String,
    /// The message's effective date: its `Date`, or the moment it arrived if that is earlier.
    /// See [`effective_date`].
    pub at: DateTime<Utc>,
    /// The key and preference its one valid `Autocrypt` header carried, if it had one.
    pub header: Option<(Fingerprint, PreferEncrypt)>,
}

/// The date the rules compare: the message's own `Date`, unless that is later than when it
/// arrived. A `Date` in the future would otherwise pin the state against every real message
/// after it (Autocrypt Level 1, "effective date").
pub fn effective_date(date: Option<DateTime<Utc>>, arrived: DateTime<Utc>) -> DateTime<Utc> {
    date.map_or(arrived, |date| date.min(arrived))
}

/// The peer state after `seen`, from the peer state before it.
///
/// - A message older than the last Autocrypt header changes nothing: it is out of order.
/// - Otherwise `last_seen` moves forward to it.
/// - A message with a header also replaces the key, the preference and `autocrypt_timestamp`.
///
/// A message with no header from someone with no state leaves them with none: there is nothing
/// to encrypt to, and a row per sender of every newsletter is not state worth keeping.
pub fn update(before: Option<AutocryptPeer>, seen: &Sighting) -> Option<AutocryptPeer> {
    let address = seen.address.trim().to_ascii_lowercase();
    let peer = match before {
        Some(peer) => peer,
        None if seen.header.is_some() => AutocryptPeer {
            address,
            last_seen: None,
            autocrypt_timestamp: None,
            key: None,
            prefer_encrypt: PreferEncrypt::NoPreference,
            gossip_timestamp: None,
            gossip_key: None,
        },
        None => return None,
    };
    if peer.autocrypt_timestamp.is_some_and(|at| seen.at < at) {
        return Some(peer);
    }
    let last_seen = Some(peer.last_seen.map_or(seen.at, |at| at.max(seen.at)));
    Some(match seen.header {
        None => AutocryptPeer { last_seen, ..peer },
        Some((key, prefer_encrypt)) => AutocryptPeer {
            last_seen,
            autocrypt_timestamp: Some(seen.at),
            key: Some(key),
            prefer_encrypt,
            ..peer
        },
    })
}

/// The peer state after a message dated `at` gossiped `key` for `address`.
///
/// Newer gossip replaces older; older gossip is ignored. Gossip never touches the peer's own
/// Autocrypt key: what someone says about you is kept apart from what you said.
pub fn gossip(
    before: Option<AutocryptPeer>,
    address: &str,
    key: Fingerprint,
    at: DateTime<Utc>,
) -> AutocryptPeer {
    let peer = before.unwrap_or_else(|| AutocryptPeer {
        address: address.trim().to_ascii_lowercase(),
        last_seen: None,
        autocrypt_timestamp: None,
        key: None,
        prefer_encrypt: PreferEncrypt::NoPreference,
        gossip_timestamp: None,
        gossip_key: None,
    });
    if peer.gossip_timestamp.is_some_and(|then| at <= then) {
        return peer;
    }
    AutocryptPeer {
        gossip_timestamp: Some(at),
        gossip_key: Some(key),
        ..peer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(n: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + n * 86_400, 0).unwrap()
    }

    const A: Fingerprint = Fingerprint::V4([0xA; 20]);
    const B: Fingerprint = Fingerprint::V4([0xB; 20]);

    fn seen(day: i64, header: Option<Fingerprint>) -> Sighting {
        Sighting {
            address: "Peer@Example.TEST".to_owned(),
            at: at(day),
            header: header.map(|key| (key, PreferEncrypt::Mutual)),
        }
    }

    #[test]
    fn a_first_header_makes_a_peer_and_a_first_plain_message_does_not() {
        assert_eq!(update(None, &seen(1, None)), None);
        let peer = update(None, &seen(1, Some(A))).unwrap();
        assert_eq!(peer.address, "peer@example.test");
        assert_eq!(peer.key, Some(A));
        assert_eq!(peer.last_seen, Some(at(1)));
        assert_eq!(peer.autocrypt_timestamp, Some(at(1)));
        assert_eq!(peer.prefer_encrypt, PreferEncrypt::Mutual);
    }

    #[test]
    fn a_newer_header_replaces_the_key_and_an_older_one_changes_nothing() {
        let peer = update(None, &seen(5, Some(A)));
        let newer = update(peer.clone(), &seen(6, Some(B))).unwrap();
        assert_eq!(newer.key, Some(B));
        assert_eq!(newer.autocrypt_timestamp, Some(at(6)));

        // Delivered late: dated before the header that is already recorded.
        let older = update(peer.clone(), &seen(4, Some(B))).unwrap();
        assert_eq!(Some(older), peer);
    }

    #[test]
    fn a_message_without_a_header_moves_last_seen_and_keeps_the_key() {
        let peer = update(None, &seen(5, Some(A)));
        let after = update(peer, &seen(9, None)).unwrap();
        assert_eq!(after.last_seen, Some(at(9)));
        assert_eq!(after.autocrypt_timestamp, Some(at(5)));
        assert_eq!(after.key, Some(A));
    }

    #[test]
    fn the_effective_date_is_never_later_than_arrival() {
        assert_eq!(effective_date(Some(at(30)), at(2)), at(2));
        assert_eq!(effective_date(Some(at(1)), at(2)), at(1));
        assert_eq!(effective_date(None, at(2)), at(2));
    }

    #[test]
    fn gossip_is_kept_apart_and_only_newer_gossip_wins() {
        let peer = update(None, &seen(5, Some(A)));
        let gossiped = gossip(peer, "peer@example.test", B, at(6));
        assert_eq!(
            gossiped.key,
            Some(A),
            "gossip never replaces the peer's own key"
        );
        assert_eq!(gossiped.gossip_key, Some(B));
        let stale = gossip(Some(gossiped.clone()), "peer@example.test", A, at(3));
        assert_eq!(stale, gossiped);
        let fresh = gossip(None, "New@Example.TEST", A, at(1));
        assert_eq!(fresh.address, "new@example.test");
        assert_eq!(fresh.last_seen, None);
    }
}
