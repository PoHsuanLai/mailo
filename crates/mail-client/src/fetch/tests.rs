use super::*;
use mail_core::fetch::Pause;
use mail_core::sync::report::{AccountReport, Counts, Trouble};
use mail_domain::id::account_id_from_uuid;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"))
}

fn finished(trouble: Vec<Trouble>) -> PassEnd {
    PassEnd::Finished(AccountReport {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        counts: Counts::default(),
        trouble,
    })
}

fn failed() -> PassEnd {
    PassEnd::Failed {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        retry: Retry::After(Duration::from_secs(5)),
        why: "cannot connect".to_owned(),
        pause: Pause::Unreachable,
        offer: None,
    }
}

fn cancelled() -> PassEnd {
    PassEnd::Cancelled {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
    }
}

fn trouble(retry: Retry, why: Option<&str>) -> Trouble {
    Trouble {
        mailbox: None,
        retry,
        why: why.map(str::to_owned),
    }
}

/// A pass that stored nothing leaves what was read alone, and one that ran or was cut short may
/// have written.
#[test]
fn what_a_pass_may_have_stored() {
    type Row = (&'static str, Result<Vec<PassEnd>, String>, bool);
    let cases: [Row; 5] = [
        ("an empty failed pass", Ok(vec![failed()]), false),
        (
            "a pass that stopped",
            Err("the sync pass stopped".to_owned()),
            false,
        ),
        ("no account ran", Ok(vec![]), false),
        ("a finished pass", Ok(vec![finished(vec![])]), true),
        ("a cancelled pass", Ok(vec![cancelled()]), true),
    ];
    for (name, pass, stored) in &cases {
        assert_eq!(may_have_stored(pass), *stored, "{name}");
    }
}

/// A fetch that ran is done even with trouble that is not a refusal; a refused sign-in, or a
/// fetch that never ran, is refused in the server's words; one cancelled is refused, so it is
/// fetched on the next opening.
#[test]
fn a_fetch_on_opening_ends_done_or_refused() {
    let slow = trouble(Retry::After(Duration::from_secs(60)), None);
    let odd = trouble(Retry::Now, Some("Archive: cannot select"));
    let refused = trouble(Retry::NeedsReauth, Some("login failed"));
    let cases = [
        (Ok(finished(Vec::new())), FolderEvent::Done),
        (Ok(finished(vec![slow, odd])), FolderEvent::Done),
        (
            Ok(finished(vec![refused])),
            FolderEvent::Refused("login failed".to_owned()),
        ),
        (
            Ok(failed()),
            FolderEvent::Refused("cannot connect".to_owned()),
        ),
        (
            Err("cannot connect".to_owned()),
            FolderEvent::Refused("cannot connect".to_owned()),
        ),
        (Ok(cancelled()), FolderEvent::Refused(String::new())),
    ];
    for (result, event) in cases {
        let said = format!("{result:?}");
        assert_eq!(ended(result), event, "{said}");
    }
}
