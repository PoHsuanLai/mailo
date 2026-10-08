use super::*;
use porter_provider::DomainName;

/// mailo's own file parses (`set` drops one that does not), and porter's mail providers are all
/// there beside it.
#[test]
fn porters_files_and_mailos_are_all_in_the_set() {
    assert_eq!(own().len(), OWN.len());
    for want in [
        "fastmail",
        "generic-imap",
        "generic-jmap",
        "gmx",
        "icloud",
        "microsoft",
        "yahoo",
        "google",
    ] {
        let id = porter_core::ProviderId::parse(want).unwrap();
        assert!(set().get(&id).is_some(), "{want}");
    }
    // mailo's Google replaces porter's, so the set is porter's count.
    assert_eq!(set().specs().len(), shipped_specs().len());
    let google = set().get(&porter_core::ProviderId::parse("google").unwrap());
    assert!(
        google.is_some_and(|spec| spec
            .capabilities
            .iter()
            .any(|c| c.family == porter_core::Family::Smtp)),
        "the Google in mailo's set is not mailo's: it has no SMTP"
    );
}

/// mailo's own file claims what mailo's preset table claimed for Google, and nothing else.
#[test]
fn the_google_file_claims_gmails_domains_and_its_exchangers() {
    use porter_provider::DomainMatch::{Domain, Mx};
    let name = |text: &str| DomainName::parse(text).unwrap();
    let claim = |domain: &str, mx: &[&str]| {
        let mx: Vec<DomainName> = mx.iter().map(|h| name(h)).collect();
        set()
            .claiming(&name(domain), &mx)
            .first()
            .map(|(spec, how)| (spec.id.to_string(), *how))
    };
    let google = |how| Some(("google".to_owned(), how));
    assert_eq!(claim("gmail.com", &[]), google(Domain));
    assert_eq!(claim("googlemail.com", &[]), google(Domain));
    assert_eq!(claim("firm.example", &["aspmx.l.google.com"]), google(Mx));
    assert_eq!(claim("firm.example", &["x.googlemail.com"]), google(Mx));
    assert_eq!(claim("sub.gmail.com", &[]), None);
    assert_eq!(claim("gmail.com.evil.test", &[]), None);
    assert_eq!(claim("firm.example", &["notgoogle.com"]), None);
    assert_eq!(claim("firm.example", &["google.com.example.test"]), None);
}
