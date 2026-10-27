//! The vacation reply as Microsoft Graph keeps it: the mailbox's `automaticRepliesSetting`,
//! changed by a `PATCH` of `/me/mailboxSettings`.
//!
//! Exchange sends it, once per sender, as it sends one set in Outlook: to people inside the
//! organisation and to everyone outside it, with the same words. What a Sieve script names and
//! Exchange has no place for is left out: Exchange writes the subject itself ("Automatic reply:"
//! and the subject answered), answers mail to any of the mailbox's addresses, and decides how
//! often. The reply's text is plain, so it goes as HTML with its characters escaped and its line
//! breaks kept.
//!
//! Exchange schedules a reply only between two instants. A reply with no start starts now, and
//! one with no end ends [`OPEN_END`] after it starts, which is as good as never; one with
//! neither is simply on.

use chrono::{DateTime, Duration, Utc};
use mail_domain::Vacation;
use serde_json::{Value, json};

/// The signed-in user's mailbox settings.
pub const MAILBOX_SETTINGS: &str = "https://graph.microsoft.com/v1.0/me/mailboxSettings";

/// How long a reply with a start and no end lasts.
pub const OPEN_END: Duration = Duration::days(3650);

/// Text as HTML that shows it as it was typed.
fn html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// An instant as Graph's `dateTimeTimeZone`, in UTC so the zone is never a guess.
fn at(instant: DateTime<Utc>) -> Value {
    json!({ "dateTime": instant.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC" })
}

/// The body of the `PATCH` that makes the mailbox's reply `vacation`, or switches it off for
/// `None`. `now` starts a reply that names no start.
pub fn automatic_replies(vacation: Option<&Vacation>, now: DateTime<Utc>) -> Value {
    let Some(vacation) = vacation else {
        return json!({ "automaticRepliesSetting": { "status": "disabled" } });
    };
    let message = html(&vacation.body);
    let mut setting = json!({
        "externalAudience": "contactsOnly",
        "internalReplyMessage": message,
        "externalReplyMessage": message,
    });
    let during = &vacation.during;
    if during.from.is_none() && during.to.is_none() {
        setting["status"] = json!("alwaysEnabled");
    } else {
        let start = during.from.unwrap_or(now);
        let end = during.to.unwrap_or(start + OPEN_END);
        setting["status"] = json!("scheduled");
        setting["scheduledStartDateTime"] = at(start);
        setting["scheduledEndDateTime"] = at(end);
    }
    json!({ "automaticRepliesSetting": setting })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_domain::DateRange;
    use mail_domain::id::new_account_id;

    fn instant(s: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(s, 0).unwrap()
    }

    fn away(from: Option<i64>, to: Option<i64>) -> Vacation {
        Vacation {
            account: new_account_id(),
            subject: "Away".to_owned(),
            body: "Back <Monday> & then\r\nsome.".to_owned(),
            days: 7,
            addresses: Vec::new(),
            from: None,
            during: DateRange {
                from: from.map(instant),
                to: to.map(instant),
            },
        }
    }

    #[test]
    fn the_setting_is_off_on_or_scheduled_between_two_instants() {
        let now = instant(1_000);
        let words = "Back &lt;Monday&gt; &amp; then<br>some.";
        let utc = |s: &str| json!({ "dateTime": s, "timeZone": "UTC" });
        let cases = [
            (None, json!({ "status": "disabled" })),
            (
                Some(away(None, None)),
                json!({
                    "status": "alwaysEnabled", "externalAudience": "contactsOnly",
                    "internalReplyMessage": words, "externalReplyMessage": words,
                }),
            ),
            (
                Some(away(Some(1_790_000_000), Some(1_790_086_400))),
                json!({
                    "status": "scheduled", "externalAudience": "contactsOnly",
                    "internalReplyMessage": words, "externalReplyMessage": words,
                    "scheduledStartDateTime": utc("2026-09-21T14:13:20"),
                    "scheduledEndDateTime": utc("2026-09-22T14:13:20"),
                }),
            ),
            // No start: it starts now.
            (
                Some(away(None, Some(86_400))),
                json!({
                    "status": "scheduled", "externalAudience": "contactsOnly",
                    "internalReplyMessage": words, "externalReplyMessage": words,
                    "scheduledStartDateTime": utc("1970-01-01T00:16:40"),
                    "scheduledEndDateTime": utc("1970-01-02T00:00:00"),
                }),
            ),
        ];
        for (vacation, setting) in cases {
            assert_eq!(
                automatic_replies(vacation.as_ref(), now),
                json!({ "automaticRepliesSetting": setting }),
                "{vacation:?}"
            );
        }
    }

    #[test]
    fn a_start_with_no_end_lasts_the_open_end() {
        let body = automatic_replies(Some(&away(Some(0), None)), instant(5));
        assert_eq!(
            body["automaticRepliesSetting"]["scheduledEndDateTime"]["dateTime"],
            (instant(0) + OPEN_END)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        );
    }
}
