//! What a group's `MEMBER` URI names (RFC 6350 §6.6.5).

/// One `MEMBER` of a group, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    /// A `mailto:` URI (RFC 6068): the addresses it is written to, decoded, in order. Its header
    /// fields (`?subject=…`) say nothing about who is in a group and are ignored.
    Mailto(Vec<String>),
    /// Any other URI, which names a card by its `UID` — a `urn:uuid:` (RFC 4122) most often, but
    /// §6.7.6 lets a `UID` be any URI, or free text. Kept as [`uid_key`] folds it, so it compares
    /// equal to the `UID` of the card it names however each was written.
    Card(String),
}

impl Member {
    /// What `uri` names.
    pub fn of(uri: &str) -> Self {
        let uri = uri.trim();
        match scheme_rest(uri, "mailto") {
            Some(rest) => {
                let to = rest.split('?').next().unwrap_or_default();
                Self::Mailto(
                    to.split(',')
                        .map(|address| percent_decode(address).trim().to_owned())
                        .filter(|address| !address.is_empty())
                        .collect(),
                )
            }
            None => Self::Card(uid_key(uri)),
        }
    }
}

/// A `UID` or a member's URI, folded so that the two spellings of one identifier compare equal:
/// `urn:uuid:` is dropped from the front (a 3.0 card often writes the bare UUID where a 4.0
/// group writes the URN) and a UUID is lower-cased, since RFC 4122 §3 reads its hex without
/// regard to case. Anything else is only trimmed.
pub fn uid_key(uid: &str) -> String {
    let uid = uid.trim();
    let bare = scheme_rest(uid, "urn")
        .and_then(|rest| scheme_rest(rest, "uuid"))
        .unwrap_or(uid);
    if is_uuid(bare) {
        bare.to_ascii_lowercase()
    } else {
        bare.to_owned()
    }
}

/// What follows `scheme:` at the start of `uri`, the scheme matched without regard to case.
fn scheme_rest<'a>(uri: &'a str, scheme: &str) -> Option<&'a str> {
    let (head, rest) = uri.split_once(':')?;
    head.eq_ignore_ascii_case(scheme).then_some(rest)
}

/// Whether `text` is a UUID in its 8-4-4-4-12 hex form.
fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `%XX` escapes decoded as UTF-8; a malformed escape is kept as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = (bytes[at] == b'%')
            .then(|| bytes.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_is_an_address_or_a_card() {
        let cases: Vec<(&str, Member)> = vec![
            (
                "mailto:ada@example.test",
                Member::Mailto(vec!["ada@example.test".into()]),
            ),
            (
                "MAILTO:ada@example.test",
                Member::Mailto(vec!["ada@example.test".into()]),
            ),
            (
                "mailto:ada%40example.test?subject=hi",
                Member::Mailto(vec!["ada@example.test".into()]),
            ),
            (
                "mailto:a@example.test,b@example.test",
                Member::Mailto(vec!["a@example.test".into(), "b@example.test".into()]),
            ),
            ("mailto:", Member::Mailto(vec![])),
            (
                "urn:uuid:03A0E51F-D1AA-4385-8A53-E29025ACD8AF",
                Member::Card("03a0e51f-d1aa-4385-8a53-e29025acd8af".into()),
            ),
            (
                "sip:ada@example.test",
                Member::Card("sip:ada@example.test".into()),
            ),
            ("%zz", Member::Card("%zz".into())),
        ];
        for (uri, want) in cases {
            assert_eq!(Member::of(uri), want, "{uri}");
        }
    }

    #[test]
    fn a_uid_matches_its_urn_whichever_way_each_is_written() {
        let bare = uid_key("03a0e51f-d1aa-4385-8a53-e29025acd8af");
        for spelled in [
            "urn:uuid:03a0e51f-d1aa-4385-8a53-e29025acd8af",
            "URN:UUID:03A0E51F-D1AA-4385-8A53-E29025ACD8AF",
            " 03A0E51F-D1AA-4385-8A53-E29025ACD8AF ",
        ] {
            assert_eq!(uid_key(spelled), bare, "{spelled}");
        }
        assert_ne!(
            uid_key("Ada-1815"),
            uid_key("ada-1815"),
            "free text keeps its case"
        );
    }

    #[test]
    fn a_malformed_escape_is_kept() {
        assert_eq!(percent_decode("a%4"), "a%4");
        assert_eq!(percent_decode("a%zz%41"), "a%zzA");
    }
}
