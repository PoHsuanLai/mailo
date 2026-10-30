//! BER made DER, so the `der` crate can read what real S/MIME clients write.
//!
//! CMS is specified in BER, and streaming encoders use it: an enveloped message from a client
//! that encrypts as it writes arrives with indefinite lengths and its ciphertext cut into a
//! constructed OCTET STRING of many pieces. The `der` crate reads DER alone. This rewrites the
//! first into the second — definite, minimal lengths, and constructed strings joined — without
//! knowing the schema.
//!
//! One thing cannot be done without the schema: a constructed context-specific tag holding only
//! OCTET STRINGs is either an IMPLICIT constructed OCTET STRING (which DER writes as one
//! primitive) or an EXPLICIT tag around one (which DER keeps). [`Implicit::Join`] reads it as the
//! first; the caller chooses it only for structures where no EXPLICIT tag wraps an OCTET STRING
//! (EnvelopedData, AuthEnvelopedData, EncryptedData), and tries plain DER first anyway.

use der::Decode;

/// How to read a constructed context-specific tag holding only OCTET STRINGs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Implicit {
    /// Leave it as it is: an EXPLICIT tag.
    Keep,
    /// Join its pieces into one primitive: an IMPLICIT OCTET STRING.
    Join,
}

/// Deepest nesting read. CMS nests a dozen levels; thousands is an attack on the stack.
const MAX_DEPTH: usize = 48;

/// `T` from `bytes`: as DER when it is, otherwise as BER rewritten, reading tags as `implicit`
/// says.
pub(crate) fn decode<T>(bytes: &[u8], implicit: Implicit) -> Option<T>
where
    T: for<'a> Decode<'a>,
{
    T::from_der(bytes)
        .ok()
        .or_else(|| T::from_der(&to_der(bytes, implicit)?).ok())
}

/// `bytes`, one BER value, as DER. `None` when it is not BER.
pub(crate) fn to_der(bytes: &[u8], implicit: Implicit) -> Option<Vec<u8>> {
    let (node, _) = read(bytes, 0)?;
    let mut out = Vec::with_capacity(bytes.len());
    write(&node, implicit, &mut out);
    Some(out)
}

/// One value read: its tag bytes, and either its content or its children.
enum Node<'a> {
    Primitive {
        tag: &'a [u8],
        content: &'a [u8],
    },
    Constructed {
        tag: &'a [u8],
        children: Vec<Node<'a>>,
    },
}

const OCTET_STRING: u8 = 0x04;
const CONSTRUCTED: u8 = 0x20;
const CONTEXT: u8 = 0x80;

/// The value at the start of `bytes`, and how many bytes it took.
fn read(bytes: &[u8], depth: usize) -> Option<(Node<'_>, usize)> {
    if depth > MAX_DEPTH {
        return None;
    }
    let first = *bytes.first()?;
    let mut at = 1;
    if first & 0x1F == 0x1F {
        // A tag number too big for five bits continues in base 128.
        loop {
            let b = *bytes.get(at)?;
            at += 1;
            if b & 0x80 == 0 {
                break;
            }
        }
    }
    let tag = &bytes[..at];
    let len_byte = *bytes.get(at)?;
    at += 1;
    let constructed = first & CONSTRUCTED != 0;
    if len_byte == 0x80 {
        // Indefinite: children until the end-of-contents octets.
        if !constructed {
            return None;
        }
        let mut children = Vec::new();
        loop {
            if bytes.get(at..at + 2)? == [0, 0] {
                return Some((Node::Constructed { tag, children }, at + 2));
            }
            let (child, used) = read(&bytes[at..], depth + 1)?;
            children.push(child);
            at += used;
        }
    }
    let length = if len_byte < 0x80 {
        usize::from(len_byte)
    } else {
        let count = usize::from(len_byte & 0x7F);
        if count > 4 {
            return None;
        }
        let mut length = 0usize;
        for _ in 0..count {
            length = (length << 8) | usize::from(*bytes.get(at)?);
            at += 1;
        }
        length
    };
    let content = bytes.get(at..at.checked_add(length)?)?;
    let end = at + length;
    if !constructed {
        return Some((Node::Primitive { tag, content }, end));
    }
    let mut children = Vec::new();
    let mut inner = 0;
    while inner < content.len() {
        let (child, used) = read(&content[inner..], depth + 1)?;
        children.push(child);
        inner += used;
    }
    Some((Node::Constructed { tag, children }, end))
}

/// The OCTET STRING pieces under `node` joined, when `node` is made of nothing else.
fn joined(children: &[Node<'_>]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for child in children {
        match child {
            Node::Primitive { tag, content } if *tag == [OCTET_STRING] => {
                out.extend_from_slice(content);
            }
            Node::Constructed { tag, children } if *tag == [OCTET_STRING | CONSTRUCTED] => {
                out.extend_from_slice(&joined(children)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

fn write(node: &Node<'_>, implicit: Implicit, out: &mut Vec<u8>) {
    match node {
        Node::Primitive { tag, content } => header(tag, content.len(), out, content),
        Node::Constructed { tag, children } => {
            let first = tag[0];
            // A constructed OCTET STRING is always joined: DER has only the primitive form.
            if *tag == [OCTET_STRING | CONSTRUCTED]
                && let Some(bytes) = joined(children)
            {
                return header(&[OCTET_STRING], bytes.len(), out, &bytes);
            }
            if implicit == Implicit::Join
                && first & 0xC0 == CONTEXT
                && tag.len() == 1
                && !children.is_empty()
                && let Some(bytes) = joined(children)
            {
                return header(&[first & !CONSTRUCTED], bytes.len(), out, &bytes);
            }
            let mut content = Vec::new();
            for child in children {
                write(child, implicit, &mut content);
            }
            header(tag, content.len(), out, &content);
        }
    }
}

/// `tag`, the DER length of `len`, then `content`.
fn header(tag: &[u8], len: usize, out: &mut Vec<u8>, content: &[u8]) {
    out.extend_from_slice(tag);
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(content);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indefinite_lengths_and_pieced_strings_become_der() {
        // SEQUENCE (indefinite) { OCTET STRING constructed (indefinite) { "ab", "c" }, INTEGER 5 }
        let ber = [
            0x30, 0x80, 0x24, 0x80, 0x04, 0x02, b'a', b'b', 0x04, 0x01, b'c', 0x00, 0x00, 0x02,
            0x01, 0x05, 0x00, 0x00,
        ];
        let der = to_der(&ber, Implicit::Keep).unwrap();
        assert_eq!(
            der,
            [0x30, 0x08, 0x04, 0x03, b'a', b'b', b'c', 0x02, 0x01, 0x05]
        );
    }

    #[test]
    fn a_context_tag_of_octet_strings_is_joined_only_when_asked() {
        let ber = [0xA0, 0x80, 0x04, 0x01, b'x', 0x04, 0x01, b'y', 0x00, 0x00];
        assert_eq!(
            to_der(&ber, Implicit::Keep).unwrap(),
            [0xA0, 0x06, 0x04, 0x01, b'x', 0x04, 0x01, b'y']
        );
        assert_eq!(
            to_der(&ber, Implicit::Join).unwrap(),
            [0x80, 0x02, b'x', b'y']
        );
    }

    #[test]
    fn long_and_non_minimal_lengths_are_rewritten_minimal() {
        let mut ber = vec![0x04, 0x82, 0x00, 0x03, 1, 2, 3];
        assert_eq!(to_der(&ber, Implicit::Keep).unwrap(), [0x04, 0x03, 1, 2, 3]);
        ber = vec![0x04, 0x82, 0x01, 0x00];
        ber.extend(std::iter::repeat_n(7u8, 256));
        let der = to_der(&ber, Implicit::Keep).unwrap();
        assert_eq!(&der[..4], [0x04, 0x82, 0x01, 0x00]);
    }

    #[test]
    fn truncated_and_absurdly_deep_input_is_refused() {
        assert!(to_der(&[0x30, 0x05, 0x02, 0x01], Implicit::Keep).is_none());
        assert!(to_der(&[0x30, 0x80, 0x02, 0x01, 0x05], Implicit::Keep).is_none());
        let deep: Vec<u8> = std::iter::repeat_n([0x30u8, 0x80], 200).flatten().collect();
        assert!(to_der(&deep, Implicit::Keep).is_none());
    }
}
