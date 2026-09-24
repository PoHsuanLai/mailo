//! The vacation reply kept and read back, its dates refused in words, and "Put on server" with
//! the server answered by the test.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{TimeZone, Utc};
use mail_domain::*;
use mail_proto::sieve::{Compiled, SieveCaps, SieveOutcome, Unmappable, VacationPlaced};
use mail_runtime::sieve::Pushed;
use mail_store::{SqliteStore, Store};

use super::away::{self, Away, Reply};
use super::server::{Push, put, reach};
use crate::ui::data::{AccountRow, account_rows};

/// A mail server of the user's own, reached with a password: one that offers ManageSieve.
pub(super) const OWN: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));

/// Add an account on `imap.nowhere.example`, as `account add` would with its hosts typed.
pub(super) fn own_server(store: &SqliteStore) -> AccountRow {
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual("me@nowhere.example", &manual, Utc::now());
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                OWN.to_string(),
                preset.plan.address.clone(),
                serde_json::to_string(&preset.plan).unwrap(),
                Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();
    store
        .put_caps(OWN, &preset.expected_caps, Utc::now())
        .unwrap();
    account_rows(store)
        .into_iter()
        .find(|row| row.id == OWN)
        .unwrap()
}

/// A fixed "now", so the words about dates do not depend on the day the test runs.
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 10, 0, 0).unwrap()
}

fn filled(until: &str) -> Away {
    Away {
        reply: Reply::On,
        subject: "Away until October".to_owned(),
        body: "Back on the 12th.\nFor anything urgent, call the office.".to_owned(),
        from: "2026-09-28".to_owned(),
        until: until.to_owned(),
        addresses: "me@nowhere.example, Me.Too@Nowhere.Example".to_owned(),
        days: Vacation::DEFAULT_DAYS,
    }
}

#[test]
fn a_reply_kept_from_the_sheet_reads_back_as_it_was_typed() {
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    let fresh = away::load(&store, &row, now(), &Utc);
    assert_eq!(fresh.reply, Reply::Off);
    assert_eq!(
        fresh.addresses, "me@nowhere.example",
        "the account's own address is not offered"
    );
    assert_eq!(store.vacation(OWN).unwrap(), None);

    let said = away::save(&store, &row, &filled("2026-10-12 09:00"), now(), &Utc).unwrap();
    assert!(
        said.starts_with("Kept: “Away until October”, from "),
        "{said}"
    );
    let kept = store.vacation(OWN).unwrap().expect("nothing was kept");
    assert_eq!(kept.subject, "Away until October");
    assert_eq!(
        kept.addresses,
        ["me@nowhere.example", "me.too@nowhere.example"],
        "addresses are compared lowercased"
    );
    assert_eq!(
        kept.during.from,
        Some(Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap())
    );
    assert_eq!(
        kept.during.to,
        Some(Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap())
    );

    let back = away::load(&store, &row, now(), &Utc);
    assert_eq!(back.reply, Reply::On);
    assert_eq!(back.subject, "Away until October");
    assert_eq!(back.body, filled("").body);
    assert_eq!(back.from, "2026-09-28 00:00");
    assert_eq!(back.until, "2026-10-12 09:00");
    assert_eq!(back.addresses, "me@nowhere.example, me.too@nowhere.example");

    // Off takes it away.
    let off = Away {
        reply: Reply::Off,
        ..back
    };
    away::save(&store, &row, &off, now(), &Utc).unwrap();
    assert_eq!(store.vacation(OWN).unwrap(), None);
}

#[test]
fn a_reply_that_has_already_ended_is_refused_in_words() {
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    let past = away::save(&store, &row, &filled("2026-09-01"), now(), &Utc);
    let why = past.unwrap_err();
    assert!(why.starts_with("“Until” is already past: "), "{why}");
    assert!(why.contains("answers nobody"), "{why}");
    assert_eq!(
        store.vacation(OWN).unwrap(),
        None,
        "a refused reply was kept"
    );

    const CASES: &[(&str, &str, &str)] = &[
        (
            "2026-10-12",
            "2026-10-01",
            "“Until” has to be after “From”.",
        ),
        ("soon", "", "“soon” is not a time"),
    ];
    for (from, until, said) in CASES {
        let form = Away {
            from: (*from).to_owned(),
            until: (*until).to_owned(),
            ..filled("")
        };
        let why = away::save(&store, &row, &form, now(), &Utc).unwrap_err();
        assert!(why.contains(said), "{from}–{until}: {why}");
    }
    // A snooze's words are a time too.
    assert_eq!(
        away::when("tomorrow", now(), &Utc).unwrap(),
        Some(Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap())
    );
    assert_eq!(away::when("", now(), &Utc).unwrap(), None);
}

/// A server that installs what it is sent: one rule it runs, one it cannot, and the reply.
pub(super) fn answering(calls: Arc<AtomicUsize>) -> Push {
    Arc::new(move |store: &SqliteStore, account, _now| {
        calls.fetch_add(1, Ordering::SeqCst);
        assert!(store.vacation(account.id).is_ok());
        Ok(Pushed {
            compiled: Compiled {
                script: String::new(),
                mapped: vec!["Bills".to_owned()],
                local_only: vec![("Tagged".to_owned(), Unmappable::Label)],
                vacation: VacationPlaced::Dated,
            },
            outcome: SieveOutcome::Installed {
                caps: SieveCaps::default(),
                displaced: None,
            },
        })
    })
}

#[test]
fn putting_rules_on_the_server_says_what_the_server_did() {
    let (store, _dir) = crate::ui::fixtures::empty();
    let row = own_server(&store);
    let calls = Arc::new(AtomicUsize::new(0));
    let said = put(&store, &row, &answering(calls.clone()), now()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        said,
        "me@nowhere.example: the server now runs 1 rule(s) and the vacation reply\n  \
         \"Tagged\" runs in this client only: a label exists only in this client\n"
    );
}

#[test]
fn an_account_whose_provider_has_no_sieve_is_never_pushed() {
    let built = crate::ui::fixtures::work();
    let rows = account_rows(&built.store);
    let refused: Vec<&AccountRow> = rows.iter().filter(|r| reach(&r.plan).is_err()).collect();
    assert!(
        !refused.is_empty(),
        "the Work Space has no Google or Microsoft account"
    );
    let calls = Arc::new(AtomicUsize::new(0));
    for row in refused {
        let why = put(&built.store, row, &answering(calls.clone()), now()).unwrap_err();
        assert!(
            why.contains("offers no ManageSieve"),
            "{}: {why}",
            row.address
        );
        let form = away::save(&built.store, row, &filled("2026-10-12"), now(), &Utc);
        assert!(
            form.is_err(),
            "{}: a reply nobody will send was kept",
            row.address
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0, "a push was attempted");
}
