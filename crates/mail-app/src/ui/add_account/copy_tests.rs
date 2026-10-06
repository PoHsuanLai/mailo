//! What the sheet says of a failure: each reason's wording and action, and that none of it
//! sends anyone to a terminal.

use mail_domain::Retry;
use porter_provider::Issuer;

use super::copy::{self, Action, Notice};
use super::flow::{Miss, Refusal, What};
use mail_core::discover::Gap;

fn no_servers(gap: Gap) -> Miss {
    Miss::NoServers {
        domain: "nowhere.test".to_owned(),
        gap,
    }
}

fn unreachable(retry: Retry) -> Miss {
    Miss::Unreachable {
        domain: "nowhere.test".to_owned(),
        retry,
        why: "no route".to_owned(),
    }
}

#[test]
fn every_miss_has_its_sentence_and_one_action() {
    let said = copy::missed(&Miss::NotAnAddress);
    assert_eq!(
        said.headline,
        "Enter a full email address, like ada@example.com."
    );
    assert_eq!((said.detail, said.action), (None, None));

    let said = copy::missed(&no_servers(Gap::Nothing));
    assert_eq!(
        said.headline,
        "Mailo couldn\u{2019}t find the mail servers for nowhere.test."
    );
    assert_eq!(said.detail, None);
    assert_eq!(said.action, Some(Action::EnterServerSettings));

    let said = copy::missed(&no_servers(Gap::StartTlsOnly));
    assert_eq!(
        said.detail.as_deref(),
        Some(
            "This provider only offers connections Mailo considers unsafe (STARTTLS). Ask them \
             for IMAP on port 993 and SMTP on port 465."
        )
    );
    assert_eq!(said.action, Some(Action::EnterServerSettings));

    let said = copy::missed(&no_servers(Gap::PersonalMicrosoft));
    assert_eq!(said.action, None);

    let said = copy::missed(&unreachable(Retry::Now));
    assert_eq!(
        said.headline,
        "Mailo couldn\u{2019}t reach the internet to look up nowhere.test."
    );
    assert_eq!(said.action, Some(Action::TryAgain));
    assert_eq!(Action::TryAgain.label(), "Try Again");
    assert_eq!(
        Action::EnterServerSettings.label(),
        "Enter Server Settings\u{2026}"
    );
}

#[test]
fn every_refusal_has_its_sentence() {
    assert_eq!(
        copy::refused(&Refusal::Blank(What::Password)).headline,
        "Type the password first."
    );
    assert_eq!(
        copy::refused(&Refusal::Blank(What::Token)).headline,
        "Type the token first."
    );
    let said = copy::refused(&Refusal::NeedsClientId(Issuer::Google));
    assert_eq!(
        said.headline,
        "Signing in with Google isn\u{2019}t set up in this build of Mailo."
    );
    assert!(said.detail.unwrap().contains("MAILO_OAUTH_CLIENT_ID"));
    assert_eq!(said.action, Some(Action::EnterServerSettings));
    let said = copy::refused(&Refusal::NeedsClientId(Issuer::Microsoft));
    assert!(said.headline.contains("Microsoft"));
    let said = copy::refused(&Refusal::Other(
        "cannot save the password: the keyring is locked".to_owned(),
    ));
    assert_eq!(said.headline, "Couldn\u{2019}t add the account.");
    assert_eq!(
        said.detail.as_deref(),
        Some("cannot save the password: the keyring is locked")
    );
    assert_eq!(said.action, None);
}

/// Text as the command line says it, which no notice may repeat.
fn cli_texts() -> Vec<String> {
    let mut all = vec![
        "no preset for \"a@b.test\". Name the servers:\n\n  mailo account add a@b.test --imap \
         imap.example.com\n\nAdd --login NAME."
            .to_owned(),
        "no JMAP session URL for a@b.test. Name it:\n\n    mailo account add a@b.test --jmap x"
            .to_owned(),
        "Run mailo account add in a terminal.".to_owned(),
        "mailo account add a@b.test".to_owned(),
    ];
    for gap in [Gap::Nothing, Gap::StartTlsOnly, Gap::PersonalMicrosoft] {
        all.push(
            mail_core::discover::Failed::NoServers {
                address: "a@b.test".to_owned(),
                gap,
                tried: "nothing".to_owned(),
            }
            .said(),
        );
    }
    all
}

fn words(notice: &Notice) -> String {
    format!(
        "{} {} {}",
        notice.headline,
        notice.detail.as_deref().unwrap_or_default(),
        notice.action.map_or("", Action::label)
    )
    .to_lowercase()
}

#[test]
fn nothing_the_sheet_can_say_sends_anyone_to_the_command_line() {
    let mut notices = Vec::new();
    for gap in [Gap::Nothing, Gap::StartTlsOnly, Gap::PersonalMicrosoft] {
        notices.push(copy::missed(&no_servers(gap)));
    }
    for retry in [
        Retry::Now,
        Retry::After(std::time::Duration::from_secs(5)),
        Retry::NeedsReauth,
        Retry::Fatal("x".to_owned()),
    ] {
        notices.push(copy::missed(&unreachable(retry)));
    }
    notices.push(copy::missed(&Miss::NotAnAddress));
    notices.push(copy::missed(&Miss::Broken(
        "cannot build an HTTP client".to_owned(),
    )));
    for what in [What::Password, What::Token] {
        notices.push(copy::refused(&Refusal::Blank(what)));
    }
    for issuer in [Issuer::Google, Issuer::Microsoft] {
        notices.push(copy::refused(&Refusal::NeedsClientId(issuer)));
        notices.push(copy::needs_client_id(issuer));
    }
    // What a failed add or lookup said in its own words, written for a terminal.
    for text in cli_texts() {
        notices.push(copy::refused(&Refusal::Other(text.clone())));
        notices.push(copy::missed(&Miss::Broken(text)));
    }
    for notice in &notices {
        let said = words(notice);
        assert!(!said.contains("mailo account"), "{said}");
        assert!(!said.contains("terminal"), "{said}");
        assert!(!said.contains("--imap"), "{said}");
    }
}
