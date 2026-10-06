use super::*;
use mail_mime::{Check, Verdict};
use mail_runtime::lookup::Miss;
use std::sync::Mutex;

/// DNS that has nothing, and remembers every name it was asked.
#[derive(Default)]
struct Counting(Mutex<Vec<String>>);

impl Txt for Counting {
    async fn txt(&self, name: &str) -> Result<Vec<String>, Miss> {
        self.0.lock().unwrap().push(name.to_owned());
        Err(Miss::Absent)
    }
}

fn results(verdict: Verdict, domain: &str) -> AuthResults {
    AuthResults {
        authserv_id: Some("mx.provider.example".to_owned()),
        spf: None,
        dkim: None,
        dmarc: Some(Check {
            verdict,
            domain: Some(domain.to_owned()),
        }),
    }
}

#[test]
fn the_old_file_is_off_unless_it_says_on() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load(dir.path()), Setting::Off, "off unless turned on");
    std::fs::write(dir.path().join(FILE_NAME), r#"{"bimi":"on"}"#).unwrap();
    assert_eq!(load(dir.path()), Setting::On);
    std::fs::write(dir.path().join(FILE_NAME), b"not json").unwrap();
    assert_eq!(load(dir.path()), Setting::Off, "an unreadable file is off");
}

#[tokio::test]
async fn nothing_is_looked_up_unless_the_setting_is_on_and_dmarc_passed_for_the_sender() {
    let cache = tempfile::tempdir().unwrap();
    let http = mail_runtime::bimi::client_builder().build().unwrap();
    let pass = results(Verdict::Pass, "brand.example");
    let fail = results(Verdict::Fail, "brand.example");
    let elsewhere = results(Verdict::Pass, "other.example");
    let cases: &[(&str, Setting, Option<&AuthResults>, &str, usize)] = &[
        ("off", Setting::Off, Some(&pass), "ada@brand.example", 0),
        ("on, no results", Setting::On, None, "ada@brand.example", 0),
        (
            "on, dmarc fail",
            Setting::On,
            Some(&fail),
            "ada@brand.example",
            0,
        ),
        (
            "on, a pass for another domain",
            Setting::On,
            Some(&elsewhere),
            "ada@brand.example",
            0,
        ),
        ("on, no domain", Setting::On, Some(&pass), "ada", 0),
        (
            "on, a pass for the sender",
            Setting::On,
            Some(&pass),
            "ada@brand.example",
            1,
        ),
    ];
    for (name, setting, results, from, asked) in cases {
        let dns = Counting::default();
        let lookup = Lookup {
            dns: &dns,
            http: &http,
            anchors: &[],
            now: chrono::Utc::now(),
        };
        let found = brand_logo(*setting, *results, from, &lookup, cache.path()).await;
        assert_eq!(found, None, "case: {name}");
        assert_eq!(dns.0.lock().unwrap().len(), *asked, "case: {name}");
        if *asked > 0 {
            assert_eq!(
                *dns.0.lock().unwrap(),
                vec!["default._bimi.brand.example"],
                "case: {name}"
            );
        }
    }
    // Off, the cache is not read either: a remembered logo stays unshown.
    mail_runtime::bimi::remember(
        cache.path(),
        "brand.example",
        Some(b"\x89PNG\r\n\x1a\nlogo"),
        chrono::Utc::now(),
    )
    .unwrap();
    let dns = Counting::default();
    let lookup = Lookup {
        dns: &dns,
        http: &http,
        anchors: &[],
        now: chrono::Utc::now(),
    };
    let on = brand_logo(
        Setting::On,
        Some(&pass),
        "ada@brand.example",
        &lookup,
        cache.path(),
    )
    .await;
    assert_eq!(
        on.as_deref(),
        Some(&b"\x89PNG\r\n\x1a\nlogo"[..]),
        "on, from the cache"
    );
    let off = brand_logo(
        Setting::Off,
        Some(&pass),
        "ada@brand.example",
        &lookup,
        cache.path(),
    )
    .await;
    assert_eq!(off, None, "off, not even the cache");
    assert!(dns.0.lock().unwrap().is_empty());
}

#[test]
fn the_roots_are_the_shipped_ones_and_the_users() {
    let dir = tempfile::tempdir().unwrap();
    let shipped = anchors(dir.path()).len();
    std::fs::write(dir.path().join(USER_ROOTS), b"not a certificate").unwrap();
    assert_eq!(
        anchors(dir.path()).len(),
        shipped,
        "a file that does not read adds none"
    );
}
