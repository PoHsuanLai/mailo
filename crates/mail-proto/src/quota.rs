//! How much of an account's storage its server says is used, and how much it allows: IMAP's
//! `QUOTA` extension (RFC 9208, read by [`crate::backend::Quota`]) and JMAP's `Quota/get`
//! (RFC 9425, read by [`crate::jmap::storage`]).
//!
//! Only storage is read. A server may count messages too, or folders, but a person who asks how
//! full the account is means its size; a limit on how many messages it holds is rare, and is
//! said by the server when it refuses one.

/// An account's storage as its server counts it, in octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Storage {
    pub used: u64,
    pub limit: u64,
}

impl Storage {
    /// How much is left before the limit, none once it is reached.
    fn left(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    /// The share of the limit in use, from 0 to 1. A limit of nothing is full.
    pub fn share(&self) -> f64 {
        if self.limit == 0 {
            return 1.0;
        }
        (self.used as f64 / self.limit as f64).min(1.0)
    }
}

/// Of the storage limits a server reported, the one the account meets first: the least left.
///
/// A mailbox can count against more than one root (the user's own and a domain's, say), and a
/// message that does not fit under either is refused, so the tightest is the one worth saying.
pub(crate) fn tightest(found: impl IntoIterator<Item = Storage>) -> Option<Storage> {
    found.into_iter().min_by_key(Storage::left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tightest_limit_is_the_one_with_least_left() {
        let cases: Vec<(Vec<Storage>, Option<Storage>)> = vec![
            (vec![], None),
            (
                vec![Storage { used: 5, limit: 10 }],
                Some(Storage { used: 5, limit: 10 }),
            ),
            (
                vec![
                    Storage {
                        used: 1,
                        limit: 100,
                    },
                    Storage {
                        used: 50,
                        limit: 60,
                    },
                ],
                Some(Storage {
                    used: 50,
                    limit: 60,
                }),
            ),
            // Over the limit is nothing left, as at it.
            (
                vec![
                    Storage {
                        used: 12,
                        limit: 10,
                    },
                    Storage { used: 0, limit: 5 },
                ],
                Some(Storage {
                    used: 12,
                    limit: 10,
                }),
            ),
        ];
        for (found, expected) in cases {
            assert_eq!(tightest(found.clone()), expected, "{found:?}");
        }
    }

    #[test]
    fn the_share_stays_between_none_and_all() {
        let cases = [
            (Storage { used: 0, limit: 10 }, 0.0),
            (Storage { used: 5, limit: 10 }, 0.5),
            (
                Storage {
                    used: 15,
                    limit: 10,
                },
                1.0,
            ),
            (Storage { used: 0, limit: 0 }, 1.0),
        ];
        for (storage, share) in cases {
            assert!(
                (storage.share() - share).abs() < f64::EPSILON,
                "{storage:?}"
            );
        }
    }
}
