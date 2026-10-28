//! The vacation reply as JMAP keeps it: the account's one `VacationResponse` (RFC 8621 §8),
//! switched on with its subject, text and dates, or off.
//!
//! The server sends it, as a Sieve script's would be sent, and decides by itself whom to answer
//! and how often: RFC 8621 leaves that to the server, so `:days`, `:addresses` and `:from`,
//! which a Sieve script names, have no place here.

use super::field::malformed;
use super::request::Call;
use super::set::SetResult;
use crate::ProtoError;
use chrono::{DateTime, SecondsFormat, Utc};
use mail_domain::{JmapAccountId, Vacation};
use serde_json::{Value, json};

/// The capability a session lists when the account has a vacation reply, and a request setting
/// it names.
pub const VACATION: &str = "urn:ietf:params:jmap:vacationresponse";

/// The id of the one `VacationResponse` an account has.
const SINGLETON: &str = "singleton";

fn date(at: Option<DateTime<Utc>>) -> Value {
    at.map_or(Value::Null, |at| {
        json!(at.to_rfc3339_opts(SecondsFormat::Secs, true))
    })
}

/// `VacationResponse/set` making the account's reply `vacation`, or switching it off for `None`.
pub fn vacation_set(account: &JmapAccountId, vacation: Option<&Vacation>, id: &str) -> Call {
    let patch = match vacation {
        Some(v) => json!({
            "isEnabled": true,
            "fromDate": date(v.during.from),
            "toDate": date(v.during.to),
            "subject": v.subject,
            "textBody": v.body,
            "htmlBody": null,
        }),
        None => json!({ "isEnabled": false }),
    };
    Call {
        name: "VacationResponse/set",
        args: json!({ "accountId": account, "update": { SINGLETON: patch } }),
        id: id.to_owned(),
    }
}

/// Whether the arguments of a `VacationResponse/set` answer say the reply was changed.
pub fn vacation_set_done(args: &Value) -> Result<(), ProtoError> {
    let result = SetResult::parse(args)?;
    if let Some((_, refused)) = result.refused.iter().find(|(id, _)| id == SINGLETON) {
        return Err(refused.proto("the vacation reply was not changed"));
    }
    if result.updated.iter().any(|id| id == SINGLETON) {
        return Ok(());
    }
    Err(malformed(
        "a VacationResponse/set answer neither changed the reply nor said why",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_domain::DateRange;
    use mail_domain::id::new_account_id;

    fn away(from: Option<i64>, to: Option<i64>) -> Vacation {
        let at = |s: i64| Utc.timestamp_opt(s, 0).unwrap();
        Vacation {
            account: new_account_id(),
            subject: "Away".to_owned(),
            body: "Back on Monday.".to_owned(),
            days: 7,
            addresses: vec!["ada@example.test".to_owned()],
            from: None,
            during: DateRange {
                from: from.map(at),
                to: to.map(at),
            },
        }
    }

    #[test]
    fn the_reply_is_the_singleton_switched_on_with_its_words_and_dates_or_off() {
        let account = JmapAccountId::from("A1".to_owned());
        let cases = [
            (
                Some(away(Some(1_790_000_000), None)),
                json!({
                    "isEnabled": true,
                    "fromDate": "2026-09-21T14:13:20Z",
                    "toDate": null,
                    "subject": "Away",
                    "textBody": "Back on Monday.",
                    "htmlBody": null,
                }),
            ),
            (None, json!({ "isEnabled": false })),
        ];
        for (vacation, patch) in cases {
            let call = vacation_set(&account, vacation.as_ref(), "v");
            assert_eq!(call.name, "VacationResponse/set");
            assert_eq!(
                call.args,
                json!({ "accountId": "A1", "update": { "singleton": patch } })
            );
        }
    }

    #[test]
    fn an_answer_says_the_reply_changed_or_why_not() {
        let cases = [
            (json!({ "updated": { "singleton": null } }), true),
            (
                json!({ "notUpdated": { "singleton": { "type": "invalidProperties" } } }),
                false,
            ),
            (json!({ "updated": {} }), false),
        ];
        for (args, done) in cases {
            assert_eq!(vacation_set_done(&args).is_ok(), done, "{args}");
        }
    }
}
