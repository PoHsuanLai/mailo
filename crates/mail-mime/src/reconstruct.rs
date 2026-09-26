//! A message put back together from the sections an IMAP server sent, minus its attachments.
//!
//! A large message is fetched as its structure, its headers and its text, and the attachments
//! stay on the server until someone opens one. What gets stored is still one RFC 5322 document —
//! so every reader, re-parse and re-sanitize that starts from the raw bytes keeps working — with
//! each attachment left behind present as a part with its own headers and an empty body.
//!
//! Which parts were left behind is written into the document itself, as headers on those parts:
//! [`REMOTE_SECTION`] and [`REMOTE_OCTETS`]. Only [`crate::parse_reconstructed`] reads them, and
//! only a document this module wrote should ever be given to it. So every header of that name
//! arriving from the server is removed here first: a sender cannot put one in a message and have
//! it believed.

use mail_domain::PartTree;
use std::collections::HashMap;

/// On a part left on the server: the IMAP section it is fetched by.
pub const REMOTE_SECTION: &str = "X-Mailo-Remote-Section";
/// On a part left on the server: its size there, before transfer decoding.
pub const REMOTE_OCTETS: &str = "X-Mailo-Remote-Octets";

/// The sections to ask for so that [`reconstruct`] can rebuild `tree`, keeping the leaves `keep`
/// accepts. `None` when `tree` is not multipart: there is nothing to leave behind, and the caller
/// fetches it whole.
pub fn sections_for(tree: &PartTree, keep: &dyn Fn(&PartTree) -> bool) -> Option<Vec<String>> {
    let PartTree::Multipart { parts, .. } = tree else {
        return None;
    };
    let mut out = vec!["HEADER".to_owned()];
    fn walk(node: &PartTree, keep: &dyn Fn(&PartTree) -> bool, out: &mut Vec<String>) {
        out.push(format!("{}.MIME", node.section()));
        match node {
            PartTree::Multipart { parts, .. } => {
                for child in parts {
                    walk(child, keep, out);
                }
            }
            PartTree::Leaf { section, .. } => {
                if keep(node) {
                    out.push(section.clone());
                }
            }
        }
    }
    for child in parts {
        walk(child, keep, &mut out);
    }
    Some(out)
}

/// Rebuild the message `tree` describes from `fetched`, which maps each section name asked for
/// by [`sections_for`] to the bytes the server sent.
///
/// A leaf whose content is in `fetched` is written out as it came; any other leaf is written with
/// its headers, the two markers, and no content. `None` when `tree` is not multipart or when a
/// header section is missing — the caller then fetches the message whole, which is always right.
pub fn reconstruct(tree: &PartTree, fetched: &HashMap<String, Vec<u8>>) -> Option<Vec<u8>> {
    let PartTree::Multipart { .. } = tree else {
        return None;
    };
    let mut out = without_markers(fetched.get("HEADER")?);
    write_children(tree, fetched, &mut out)?;
    Some(out)
}

/// A multipart's children, each behind its delimiter, and the closing delimiter.
fn write_children(
    node: &PartTree,
    fetched: &HashMap<String, Vec<u8>>,
    out: &mut Vec<u8>,
) -> Option<()> {
    let PartTree::Multipart {
        boundary, parts, ..
    } = node
    else {
        return None;
    };
    for child in parts {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let header = without_markers(fetched.get(&format!("{}.MIME", child.section()))?);
        match child {
            PartTree::Multipart { .. } => {
                out.extend_from_slice(&header);
                write_children(child, fetched, out)?;
            }
            PartTree::Leaf {
                section, octets, ..
            } => match fetched.get(section) {
                Some(content) => {
                    out.extend_from_slice(&header);
                    out.extend_from_slice(content);
                }
                None => {
                    let marks =
                        format!("{REMOTE_SECTION}: {section}\r\n{REMOTE_OCTETS}: {octets}\r\n");
                    out.extend_from_slice(&with_headers(&header, marks.as_bytes()));
                }
            },
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Some(())
}

/// Whether `raw` is a message [`reconstruct`] rebuilt with parts left on the server: stored
/// bytes that are not the message as it was sent, only its headers and text.
///
/// Read from the bytes, not from [`mail_domain::Message::attachments`]: fetching a part later
/// marks that attachment held but leaves the stored document as it was rebuilt, with an empty
/// body where the part was. So a message whose every attachment has since been downloaded is
/// still a rebuilt one here, which it is.
///
/// A sender can write the marker headers into an ordinary message, and then this says yes of
/// a message that is whole. That errs the safe way: every caller treats yes as "these bytes may
/// not be the message as sent", and refuses or says so.
pub fn left_on_server(raw: &[u8]) -> bool {
    crate::parse_reconstructed(raw)
        .is_ok_and(|parsed| parsed.attachments.iter().any(|part| part.remote.is_some()))
}

/// One part's content, transfer-decoded: its `N.MIME` header and its `N` bytes as the server
/// sent them, which are still base64 or quoted-printable.
///
/// Decoded exactly as a whole message's attachments are — by parsing the part as a message of
/// its own — so a part fetched later is the same bytes it would have been fetched whole.
pub fn decode_part(mime_header: &[u8], content: &[u8]) -> Vec<u8> {
    let mut doc = mime_header.to_vec();
    doc.extend_from_slice(content);
    mail_parser::MessageParser::default()
        .parse(&doc)
        .and_then(|message| message.parts.first().map(|part| part.contents().to_vec()))
        .unwrap_or_else(|| content.to_vec())
}

/// A header block with every field named like one of ours removed, continuation lines included.
///
/// Matched as a whole field name, case-insensitively: `X-Mailo-Remote-Section-Extra` is some
/// other header and stays.
fn without_markers(block: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(block.len());
    let mut dropping = false;
    for line in block.split_inclusive(|b| *b == b'\n') {
        let continuation = matches!(line.first(), Some(b' ' | b'\t'));
        if !continuation {
            dropping = is_marker(line);
        }
        if !dropping {
            out.extend_from_slice(line);
        }
    }
    out
}

fn is_marker(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|b| *b == b':') else {
        return false;
    };
    let name = String::from_utf8_lossy(&line[..colon]);
    let name = name.trim();
    [REMOTE_SECTION, REMOTE_OCTETS]
        .iter()
        .any(|ours| name.eq_ignore_ascii_case(ours))
}

/// `extra` inserted as the last fields of a header block, before the empty line that ends it.
fn with_headers(block: &[u8], extra: &[u8]) -> Vec<u8> {
    let end = [&b"\r\n\r\n"[..], b"\n\n"]
        .iter()
        .find_map(|blank| {
            block
                .windows(blank.len())
                .rposition(|w| w == *blank)
                .map(|at| at + blank.len() / 2)
        })
        .unwrap_or(block.len());
    let mut out = block[..end].to_vec();
    if !out.is_empty() && !out.ends_with(b"\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(extra);
    out.extend_from_slice(b"\r\n");
    out
}
