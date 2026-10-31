use super::server::instant;

#[test]
fn a_date_is_the_start_of_that_day_in_the_readers_zone() {
    let taipei = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    let cases = [
        ("2026-10-08", "2026-10-07T16:00:00+00:00"),
        ("2026-10-08T09:00", "2026-10-08T01:00:00+00:00"),
        ("2026-10-08 09:30", "2026-10-08T01:30:00+00:00"),
    ];
    for (text, want) in cases {
        assert_eq!(instant(text, &taipei).unwrap().to_rfc3339(), want, "{text}");
    }
    assert!(instant("next week", &taipei).is_err());
}

#[test]
fn a_vacation_reply_goes_where_the_account_s_server_keeps_one() {
    use super::server::{AwayServer, away_server};
    use mail_domain::presets;
    let now = chrono::Utc::now();
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let jmap = presets::jmap(
        "me@jmap.example",
        "https://jmap.example/.well-known/jmap",
        mail_domain::HttpAuth::Basic,
    );
    let organisation = presets::microsoft_for(
        presets::MicrosoftAccount::Organisation,
        "me@contoso.example",
        now,
    );
    let personal =
        presets::microsoft_for(presets::MicrosoftAccount::Personal, "me@outlook.com", now);
    let cases = [
        (
            presets::manual("me@nowhere.example", &manual, now).plan,
            Some(AwayServer::Sieve),
        ),
        (jmap.plan, Some(AwayServer::Jmap)),
        (organisation.plan, Some(AwayServer::Graph)),
        // Over IMAP, Microsoft has no ManageSieve and the reply is not set through Graph.
        (personal.plan, None),
    ];
    for (plan, on) in cases {
        assert_eq!(away_server(&plan).ok(), on, "{:?}", plan.incoming);
    }
}
