//! Quotas (RFC 9425): how much of the account's storage its mail uses, from `Quota/get`.
//!
//! A quota counts octets or a number of objects, over one or more data types (`Mail`,
//! `Calendar`, …), for the account alone or for its whole domain. Only those counting octets
//! for mail, or for every type, say how full the mail account is; of them the tightest is the
//! one said ([`crate::quota`]).

use super::field::{malformed, object, strings, unsigned};
use super::request::Call;
use crate::ProtoError;
use crate::quota::{Storage, tightest};
use mail_domain::JmapAccountId;
use serde_json::{Value, json};

/// The capability a session lists when it has quotas, and a request using them names.
pub const QUOTA: &str = "urn:ietf:params:jmap:quota";

/// `Quota/get` for every quota on the account: there are a handful at most.
pub fn quota_get(account: &JmapAccountId, id: &str) -> Call {
    Call {
        name: "Quota/get",
        args: json!({ "accountId": account, "ids": null }),
        id: id.to_owned(),
    }
}

/// The account's mail storage, from the arguments of a `Quota/get` answer: `None` when no
/// quota counts the octets of its mail.
pub fn storage(args: &Value) -> Result<Option<Storage>, ProtoError> {
    let list = match args.get("list") {
        Some(Value::Array(list)) => list,
        _ => return Err(malformed("a Quota/get answer has no list")),
    };
    let mut found = Vec::new();
    for quota in list {
        object(quota, "a quota")?;
        if quota.get("resourceType").and_then(Value::as_str) != Some("octets") {
            continue;
        }
        // No types listed is every type, mail with them.
        let types = strings(quota, "types")?;
        if !types.is_empty() && !types.iter().any(|t| t == "Mail") {
            continue;
        }
        // A quota with no hard limit, or a limit of nothing, limits nothing: it is not "full".
        let Some(limit) = quota
            .get("hardLimit")
            .and_then(Value::as_u64)
            .filter(|limit| *limit > 0)
        else {
            continue;
        };
        found.push(Storage {
            used: unsigned(quota, "used", 0)?,
            limit,
        });
    }
    Ok(tightest(found))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(resource: &str, types: Value, used: u64, limit: u64) -> Value {
        json!({
            "id": "q",
            "resourceType": resource,
            "types": types,
            "scope": "account",
            "name": "a quota",
            "used": used,
            "hardLimit": limit,
        })
    }

    #[test]
    fn a_quota_with_no_hard_limit_is_not_storage() {
        let unlimited = json!({
            "id": "q", "resourceType": "octets", "types": ["Mail"], "scope": "account",
            "name": "a quota", "used": 10,
        });
        let zero = quota("octets", json!(["Mail"]), 10, 0);
        let args = json!({ "list": [unlimited, zero] });
        assert_eq!(storage(&args).unwrap(), None);
    }

    #[test]
    fn only_octets_counted_for_mail_are_storage() {
        let cases: Vec<(Vec<Value>, Option<Storage>)> = vec![
            (vec![], None),
            (
                vec![quota("octets", json!(["Mail"]), 10, 100)],
                Some(Storage {
                    used: 10,
                    limit: 100,
                }),
            ),
            // A count of messages is not storage.
            (vec![quota("count", json!(["Mail"]), 10, 100)], None),
            // Nor is a calendar's.
            (vec![quota("octets", json!(["Calendar"]), 10, 100)], None),
            // No types is every type.
            (
                vec![quota("octets", json!([]), 10, 100)],
                Some(Storage {
                    used: 10,
                    limit: 100,
                }),
            ),
            // The account's own and its domain's: the one with less left.
            (
                vec![
                    quota("octets", json!(["Mail"]), 10, 100),
                    quota("octets", json!(["Mail", "Contact"]), 950, 1000),
                ],
                Some(Storage {
                    used: 950,
                    limit: 1000,
                }),
            ),
        ];
        for (list, expected) in cases {
            let args = json!({ "accountId": "a", "state": "s", "list": list, "notFound": [] });
            assert_eq!(storage(&args).unwrap(), expected, "{list:?}");
        }
    }

    #[test]
    fn a_stranger_s_shape_is_an_error_not_a_guess() {
        for args in [
            json!({ "state": "s" }),
            json!({ "list": [1] }),
            json!({ "list": [{ "resourceType": "octets", "used": "lots", "hardLimit": 100 }] }),
        ] {
            assert!(storage(&args).is_err(), "{args}");
        }
    }

    #[test]
    fn the_call_asks_for_every_quota_of_the_account() {
        let call = quota_get(&JmapAccountId::from("a1".to_owned()), "q");
        assert_eq!(call.name, "Quota/get");
        assert_eq!(call.args, json!({ "accountId": "a1", "ids": null }));
    }
}
