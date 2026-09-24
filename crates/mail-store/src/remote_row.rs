//! A [`RemoteRef`] as the four `remote_map` columns, and back.
//!
//! One place, shared by both stores, because the decoding was written out six times — and a
//! third protocol would have had to be added to each of them, with any one missed reading its
//! rows as POP3.
//!
//! | protocol | `mailbox`     | `uidvalidity`  | `uid`   | `uidl`      |
//! |----------|---------------|----------------|---------|-------------|
//! | IMAP     | folder path   | UIDVALIDITY    | UID     | NULL        |
//! | POP3     | `INBOX`       | NULL           | NULL    | UIDL        |
//! | Graph    | folder path   | [`GRAPH`]      | NULL    | Graph's id  |
//!
//! Graph's id goes where a UIDL goes because the table's `CHECK` wants exactly one of `uid` and
//! `uidl`, and both are opaque strings the server chose. What tells the two apart is
//! `uidvalidity`, which a POP3 row never has and a Graph row always has, set to a value no IMAP
//! server can send — so no row changes meaning, and no migration rebuilds the table.

use crate::StoreError;
use mail_domain::RemoteRef;

/// The `uidvalidity` of a Graph row. Negative, where every real one is a `u32`.
pub(crate) const GRAPH: i64 = -2;

/// The addressing columns of one `remote_map` row: mailbox, uidvalidity, uid, uidl.
pub(crate) type Columns = (String, Option<i64>, Option<i64>, Option<String>);

/// Where `remote` is written.
pub(crate) fn columns(remote: &RemoteRef) -> Columns {
    match remote {
        RemoteRef::Imap {
            mailbox,
            uidvalidity,
            uid,
        } => (
            mailbox.clone(),
            Some(i64::from(*uidvalidity)),
            Some(i64::from(*uid)),
            None,
        ),
        RemoteRef::Pop { uidl } => ("INBOX".to_owned(), None, None, Some(uidl.clone())),
        RemoteRef::Graph { mailbox, id } => (mailbox.clone(), Some(GRAPH), None, Some(id.clone())),
    }
}

/// The address a row holds.
pub(crate) fn remote((mailbox, uidvalidity, uid, uidl): Columns) -> Result<RemoteRef, StoreError> {
    match (uidvalidity, uid, uidl) {
        (_, Some(uid), None) => Ok(RemoteRef::Imap {
            mailbox,
            uidvalidity: uidvalidity.unwrap_or(0) as u32,
            uid: uid as u32,
        }),
        (Some(GRAPH), None, Some(id)) => Ok(RemoteRef::Graph { mailbox, id }),
        (_, None, Some(uidl)) => Ok(RemoteRef::Pop { uidl }),
        _ => Err(StoreError::Decode {
            what: "remote_map row".to_owned(),
            why: "row has neither a uid nor a uidl".to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_protocol_reads_back_as_itself() {
        for remote in [
            RemoteRef::Imap {
                mailbox: "Archive".to_owned(),
                uidvalidity: 7,
                uid: 42,
            },
            RemoteRef::Pop {
                uidl: "UID-1".to_owned(),
            },
            RemoteRef::Graph {
                mailbox: "INBOX".to_owned(),
                id: "AAMk/a+b==".to_owned(),
            },
        ] {
            assert_eq!(remote_of(&remote), remote);
        }
    }

    #[test]
    fn a_pop_row_is_not_read_as_graph() {
        assert_eq!(
            remote(("INBOX".to_owned(), None, None, Some("x".to_owned()))).unwrap(),
            RemoteRef::Pop {
                uidl: "x".to_owned()
            }
        );
    }

    fn remote_of(r: &RemoteRef) -> RemoteRef {
        remote(columns(r)).unwrap()
    }
}
