//! `Authentication-Results` (RFC 8601), read from whole messages: which field is believed, and
//! what it says.
//!
//! The receiving provider here is `provider.example`, whose servers write `mx.provider.example`
//! as their authserv-id. Every field a sender could have written sits below the provider's, the
//! way a receiving server's prepended fields sit above the ones that arrived.

use mail_mime::{AuthResults, Check, Receiver, Verdict, authentication_results};

fn message(headers: &str) -> Vec<u8> {
    format!(
        "Received: from out.sender.example by mx.provider.example; Fri, 25 Sep 2026 10:00:00 +0000\r\n\
         {headers}\
         From: Ada <ada@sender.example>\r\nTo: me@provider.example\r\nSubject: hello\r\n\r\n\
         Authentication-Results: mx.provider.example; dkim=pass header.d=body.example\r\n"
    )
    .into_bytes()
}

fn provider() -> Receiver {
    Receiver::Domains(vec!["provider.example".to_owned()])
}

/// (spf, dkim, dmarc) verdicts, `None` for a method the field does not mention.
type Said = (Option<Verdict>, Option<Verdict>, Option<Verdict>);

fn said(results: &AuthResults) -> Said {
    let verdict = |check: &Option<Check>| check.as_ref().map(|c| c.verdict);
    (
        verdict(&results.spf),
        verdict(&results.dkim),
        verdict(&results.dmarc),
    )
}

use Verdict::*;

/// `(name, headers, receiver, expected)`. `None` expected: no field is believed.
fn cases() -> Vec<(&'static str, &'static str, Receiver, Option<Said>)> {
    vec![
        (
            "all three pass",
            "Authentication-Results: mx.provider.example;\r\n \
             spf=pass (sender IP is 192.0.2.1) smtp.mailfrom=ada@sender.example;\r\n \
             dkim=pass header.i=@sender.example header.s=s1;\r\n \
             dmarc=pass (p=REJECT) header.from=sender.example\r\n",
            provider(),
            Some((Some(Pass), Some(Pass), Some(Pass))),
        ),
        (
            "all three fail",
            "Authentication-Results: mx.provider.example; spf=fail smtp.mailfrom=sender.example;\r\n \
             dkim=fail reason=\"signature did not verify\" header.d=sender.example;\r\n \
             dmarc=fail header.from=sender.example\r\n",
            provider(),
            Some((Some(Fail), Some(Fail), Some(Fail))),
        ),
        (
            "results of none",
            "Authentication-Results: mx.provider.example; spf=none smtp.mailfrom=sender.example;\r\n \
             dkim=none; dmarc=none header.from=sender.example\r\n",
            provider(),
            Some((Some(None), Some(None), Some(None))),
        ),
        (
            "a field that checked nothing",
            "Authentication-Results: mx.provider.example 1; none\r\n",
            provider(),
            Some((Option::None, Option::None, Option::None)),
        ),
        (
            "softfail, temperror, neutral, and mixed case",
            "Authentication-Results: MX.Provider.Example; SPF=SoftFail smtp.mailfrom=sender.example;\r\n \
             dkim = temperror header.d = sender.example; dmarc=Neutral\r\n",
            provider(),
            Some((Some(SoftFail), Some(TempError), Some(Neutral))),
        ),
        (
            "several fields: an internal hop above the provider's is not the provider",
            "Authentication-Results: relay.elsewhere.example; dmarc=fail\r\n\
             Authentication-Results: mx.provider.example; spf=pass smtp.mailfrom=sender.example;\r\n \
             dkim=pass header.d=sender.example; dmarc=pass header.from=sender.example\r\n",
            provider(),
            Some((Some(Pass), Some(Pass), Some(Pass))),
        ),
        (
            "a forged pass below the trusted fail is ignored",
            "Authentication-Results: mx.provider.example; spf=fail smtp.mailfrom=sender.example;\r\n \
             dkim=none; dmarc=fail header.from=sender.example\r\n\
             DKIM-Signature: v=1; d=sender.example; s=s1; b=AAAA\r\n\
             Authentication-Results: mx.provider.example; spf=pass; dkim=pass; dmarc=pass\r\n",
            provider(),
            Some((Some(Fail), Some(None), Some(Fail))),
        ),
        (
            "a sender's own field, when the provider wrote none, is never believed",
            "Authentication-Results: mx.sender.example; spf=pass; dkim=pass; dmarc=pass\r\n",
            provider(),
            Option::None,
        ),
        (
            "a look-alike authserv-id is somebody else",
            "Authentication-Results: mx.evil-provider.example; spf=pass; dkim=pass; dmarc=pass\r\n\
             Authentication-Results: provider.example.evil.example; dmarc=pass\r\n",
            provider(),
            Option::None,
        ),
        (
            "a pass hidden in a comment or a quoted string is not a result",
            "Authentication-Results: mx.provider.example; spf=fail (dmarc=pass; dkim=pass)\r\n \
             smtp.mailfrom=\"x; dmarc=pass\"; dmarc=fail header.from=sender.example\r\n",
            provider(),
            Some((Some(Fail), Option::None, Some(Fail))),
        ),
        (
            "an encoded word is not decoded into a result",
            "Authentication-Results: mx.provider.example; dmarc=fail\r\n \
             header.from==?utf-8?q?x=3B_dkim=3Dpass?=\r\n",
            provider(),
            Some((Option::None, Option::None, Some(Fail))),
        ),
        (
            "several DKIM signatures: one passing is a pass",
            "Authentication-Results: mx.provider.example; dkim=fail header.d=other.example;\r\n \
             dkim=pass header.d=sender.example; dkim=neutral header.d=third.example\r\n",
            provider(),
            Some((Option::None, Some(Pass), Option::None)),
        ),
        (
            "a version this reader does not know is not read",
            "Authentication-Results: mx.provider.example 2; dmarc=pass\r\n",
            provider(),
            Option::None,
        ),
        (
            "with the receiver unknown, only the topmost field is read",
            "Authentication-Results: mx.somewhere.example; dmarc=fail header.from=sender.example\r\n\
             Authentication-Results: mx.somewhere.example; dmarc=pass header.from=sender.example\r\n",
            Receiver::Topmost,
            Some((Option::None, Option::None, Some(Fail))),
        ),
        (
            "with the receiver unknown, a field with no authserv-id is read",
            "Authentication-Results: spf=pass (sender IP is 192.0.2.1)\r\n \
             smtp.mailfrom=sender.example; dkim=pass (signature was verified)\r\n \
             header.d=sender.example;dmarc=pass action=none header.from=sender.example\r\n",
            Receiver::Topmost,
            Some((Some(Pass), Some(Pass), Some(Pass))),
        ),
        (
            "a field with no authserv-id never matches a known receiver",
            "Authentication-Results: spf=pass smtp.mailfrom=sender.example; dmarc=pass\r\n",
            provider(),
            Option::None,
        ),
        ("no field at all", "", provider(), Option::None),
        (
            "no field at all, receiver unknown",
            "",
            Receiver::Topmost,
            Option::None,
        ),
    ]
}

#[test]
fn the_believed_field_says_what_it_says() {
    for (name, headers, receiver, want) in cases() {
        let got = authentication_results(&message(headers), &receiver);
        assert_eq!(got.as_ref().map(said), want, "case: {name}\n{got:#?}");
    }
}

#[test]
fn the_domains_each_check_was_about_are_kept() {
    let raw = message(
        "Authentication-Results: mx.provider.example;\r\n \
         spf=pass smtp.mailfrom=bounce@Mail.Sender.Example;\r\n \
         dkim=pass header.i=@signer.example; dmarc=pass header.from=sender.example\r\n",
    );
    let got = authentication_results(&raw, &provider()).expect("the provider's field is read");
    assert_eq!(got.authserv_id.as_deref(), Some("mx.provider.example"));
    let domain = |check: &Option<Check>| check.as_ref().and_then(|c| c.domain.clone());
    assert_eq!(domain(&got.spf).as_deref(), Some("mail.sender.example"));
    assert_eq!(domain(&got.dkim).as_deref(), Some("signer.example"));
    assert_eq!(domain(&got.dmarc).as_deref(), Some("sender.example"));
}

#[test]
fn hostile_bytes_are_never_a_panic() {
    let inputs: &[&[u8]] = &[
        b"",
        b"\0\0\0",
        b"Authentication-Results:\r\n\r\n",
        b"Authentication-Results: ;;;;=\r\n\r\n",
        b"Authentication-Results: (((((\r\n\r\n",
        b"Authentication-Results: \"unterminated; dkim=pass\r\n\r\n",
        b"Authentication-Results: a; =pass; dkim=; spf\r\n\r\n",
        b"Authentication-Results: \xff\xfe; dkim=\xffpass\r\n\r\n",
    ];
    for raw in inputs {
        let _ = authentication_results(raw, &Receiver::Topmost);
        let _ = authentication_results(raw, &provider());
    }
}
