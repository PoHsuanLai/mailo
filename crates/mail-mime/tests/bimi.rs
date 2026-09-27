//! A Verified Mark Certificate vouches for a logo only when every part of it holds: the chain,
//! the dates, the purpose, the domain and the logo's hash. Against a throwaway authority made in
//! `bimi_support`.

mod bimi_support;

use bimi_support::*;
use mail_mime::bimi::{MarkProblem, same_logo, verified_logo};

#[test]
fn a_mark_certificate_vouches_for_its_logo_only_when_all_of_it_holds() {
    let trusted = authority("Trusted", 10);
    let stranger = authority("Stranger", 20);
    let anchors = [trusted.root.clone()];
    let domains: &[&str] = &["brand.example"];
    let logo = LOGO.as_bytes();

    let good = Mark::new(domains, logo);
    type Case<'a> = (&'a str, Vec<u8>, &'a str, Result<Vec<u8>, MarkProblem>);
    let cases: Vec<Case> = vec![
        (
            "good",
            pem(&trusted, &good),
            "brand.example",
            Ok(logo.to_vec()),
        ),
        (
            "good, domain in another case",
            pem(&trusted, &good),
            "Brand.Example",
            Ok(logo.to_vec()),
        ),
        (
            "another domain",
            pem(&trusted, &good),
            "evil-brand.example",
            Err(MarkProblem::NotForDomain),
        ),
        (
            "a parent of the named domain",
            pem(&trusted, &good),
            "example",
            Err(MarkProblem::NotForDomain),
        ),
        (
            "an authority nobody trusts",
            pem(&stranger, &good),
            "brand.example",
            Err(MarkProblem::Untrusted),
        ),
        (
            "a server certificate",
            pem(
                &trusted,
                &Mark {
                    purposes: vec![SERVER_AUTH],
                    ..Mark::new(domains, logo)
                },
            ),
            "brand.example",
            Err(MarkProblem::NotForMarks),
        ),
        (
            "expired",
            pem(
                &trusted,
                &Mark {
                    valid: (at(2024, 1, 1), at(2025, 1, 1)),
                    ..Mark::new(domains, logo)
                },
            ),
            "brand.example",
            Err(MarkProblem::Dated),
        ),
        (
            "a hash for another logo",
            pem(
                &trusted,
                &Mark {
                    stated_hash: Some(vec![0u8; 32]),
                    ..Mark::new(domains, logo)
                },
            ),
            "brand.example",
            Err(MarkProblem::LogoHash),
        ),
        (
            "not a certificate",
            b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n".to_vec(),
            "brand.example",
            Err(MarkProblem::Unreadable),
        ),
        (
            "empty",
            Vec::new(),
            "brand.example",
            Err(MarkProblem::Unreadable),
        ),
    ];
    for (name, pem, domain, want) in cases {
        assert_eq!(
            verified_logo(&pem, domain, &anchors, now()),
            want,
            "case: {name}"
        );
    }
}

#[test]
fn the_fetched_logo_must_be_the_certified_one() {
    let logo = LOGO.as_bytes();
    assert!(same_logo(logo, logo));
    let other = LOGO.replace("#1a73e8", "#e81a1a");
    assert!(!same_logo(other.as_bytes(), logo));
}
