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

#[test]
fn a_pass_that_stored_nothing_leaves_the_window_alone() {
    assert!(
        !may_have_stored(&Ok(vec![failed()])),
        "an empty failed pass"
    );
    assert!(!may_have_stored(&Err("the sync pass stopped".to_owned())));
    assert!(!may_have_stored(&Ok(vec![])), "no account ran");
}

#[test]
fn a_pass_that_ran_or_was_cut_short_is_read_again() {
    assert!(may_have_stored(&Ok(vec![finished()])));
    assert!(may_have_stored(&Ok(vec![cancelled()])));
}
