use super::*;
use mail_core::fetch::Pause;
use mail_core::sync::report::{AccountReport, Counts};
use mail_domain::Retry;
use mail_domain::id::account_id_from_uuid;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"))
}

fn finished() -> PassEnd {
    PassEnd::Finished(AccountReport {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        counts: Counts::default(),
        trouble: vec![],
    })
}

fn failed() -> PassEnd {
    PassEnd::Failed {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
        retry: Retry::After(Duration::from_secs(5)),
        why: "cannot connect".to_owned(),
        pause: Pause::Unreachable,
    }
}

fn cancelled() -> PassEnd {
    PassEnd::Cancelled {
        account: acct_account(),
        address: "me@nowhere.example".to_owned(),
    }
}

/// Whether the window reads the store again after a pass: a pass that stored nothing leaves the
/// window alone, and one that ran or was cut short is read again.
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
        ("a finished pass", Ok(vec![finished()]), true),
        ("a cancelled pass", Ok(vec![cancelled()]), true),
    ];
    for (name, pass, read_again) in &cases {
        assert_eq!(may_have_stored(pass), *read_again, "{name}");
    }
}
