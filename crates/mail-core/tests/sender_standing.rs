//! What the receiving server's checks come to, for the shapes mail really arrives in.
//!
//! Each sample is the `From` a message shows and the `Authentication-Results` its provider
//! wrote, set against the word the reader should show: Passed (the quiet check mark), Failed
//! ("May not be from this sender") or Unsure (nothing). The rule is DMARC's (RFC 7489): a pass
//! counts when it was about the sender's own domain, either check is enough, and only the
//! domain's own word that a server is not its is a warning. Forwarding, fetching into another
//! mailbox and mailing lists break SPF or DKIM routinely; they must not read as forgery.

use mail_core::auth::{Standing, standing};
use mail_mime::{Receiver, authentication_results};

struct Sample {
    name: &'static str,
    from: &'static str,
    /// The provider's `Authentication-Results`, after its authserv-id.
    results: &'static str,
    want: Standing,
}

const SAMPLES: &[Sample] = &[
    // The domain's own verdict.
    Sample {
        name: "DMARC passed",
        from: "Shop <orders@shop.example>",
        results: "spf=pass smtp.mailfrom=shop.example; dkim=pass header.d=shop.example; \
                  dmarc=pass header.from=shop.example",
        want: Standing::Passed,
    },
    Sample {
        name: "DMARC failed, whatever else passed",
        from: "Bank <alerts@bank.example>",
        results: "spf=pass smtp.mailfrom=attacker.example; dkim=pass header.d=attacker.example; \
                  dmarc=fail header.from=bank.example",
        want: Standing::Failed,
    },
    // Forwarded, or fetched into another mailbox: SPF judges the server that handed it on.
    Sample {
        name: "fetched into another mailbox: SPF softfails, the sender's signature passes",
        from: "Code Host <noreply@codehost.example>",
        results: "spf=softfail smtp.mailfrom=noreply@codehost.example; \
                  dkim=pass header.i=@codehost.example header.s=s2026",
        want: Standing::Passed,
    },
    Sample {
        name: "forwarded: SPF fails outright, the sender's signature passes",
        from: "Shop <orders@shop.example>",
        results: "spf=fail smtp.mailfrom=shop.example; dkim=pass header.d=shop.example",
        want: Standing::Passed,
    },
    Sample {
        name: "forwarded, unsigned: SPF softfails and nothing else was checked",
        from: "Friend <friend@home.example>",
        results: "spf=softfail smtp.mailfrom=home.example; dkim=none",
        want: Standing::Unsure,
    },
    // A mailing list: it rewrites the body, breaking the signature, and sends from its own server.
    Sample {
        name: "through a mailing list",
        from: "Ada <ada@member.example>",
        results: "spf=pass smtp.mailfrom=bounces@lists.example; \
                  dkim=fail reason=\"body hash did not verify\" header.d=member.example",
        want: Standing::Unsure,
    },
    // A mailing service sending for a brand.
    Sample {
        name: "a mailing service that signs only as itself",
        from: "Brand <news@brand.example>",
        results: "spf=pass smtp.mailfrom=bounce@mailer-service.example; \
                  dkim=pass header.d=mailer-service.example",
        want: Standing::Unsure,
    },
    Sample {
        name: "a mailing service that signs as itself and as the brand",
        from: "Brand <news@brand.example>",
        results: "spf=pass smtp.mailfrom=bounce@mailer-service.example; \
                  dkim=pass header.d=mailer-service.example; dkim=pass header.d=brand.example",
        want: Standing::Passed,
    },
    Sample {
        name: "the brand's own bounce domain under it",
        from: "Brand <news@brand.example>",
        results: "spf=pass smtp.mailfrom=bounce@em.brand.example; dkim=none",
        want: Standing::Passed,
    },
    // Whose domain it is.
    Sample {
        name: "a subdomain sender, signed by its parent",
        from: "Brand <hello@news.brand.example>",
        results: "dkim=pass header.d=brand.example",
        want: Standing::Passed,
    },
    Sample {
        name: "a lookalike domain that ends in the sender's name",
        from: "Brand <hello@brand.example>",
        results: "dkim=pass header.d=brand.example.evil.test; spf=pass smtp.mailfrom=brand.example.evil.test",
        want: Standing::Unsure,
    },
    Sample {
        name: "one of several signatures fails, the sender's passes",
        from: "Shop <orders@shop.example>",
        results: "dkim=fail header.d=shop.example; dkim=pass header.d=shop.example",
        want: Standing::Passed,
    },
    Sample {
        name: "the sender's server passes SPF though the signature broke",
        from: "Shop <orders@shop.example>",
        results: "spf=pass smtp.mailfrom=shop.example; dkim=fail header.d=shop.example",
        want: Standing::Passed,
    },
    // Forgery.
    Sample {
        name: "the sender's domain says the server is not its, and nothing vouches",
        from: "Bank <alerts@bank.example>",
        results: "spf=fail smtp.mailfrom=bank.example; dkim=none",
        want: Standing::Failed,
    },
    Sample {
        name: "an SPF fail about a bounce domain that is not the sender's",
        from: "Shop <orders@shop.example>",
        results: "spf=fail smtp.mailfrom=bounce@relay.example; dkim=none",
        want: Standing::Unsure,
    },
    // No verdict from DMARC, so the checks decide.
    Sample {
        name: "the domain publishes no DMARC policy",
        from: "Shop <orders@shop.example>",
        results: "spf=pass smtp.mailfrom=shop.example; dkim=none; dmarc=none header.from=shop.example",
        want: Standing::Passed,
    },
    Sample {
        name: "DMARC could not be checked just then",
        from: "Shop <orders@shop.example>",
        results: "dkim=pass header.d=shop.example; dmarc=temperror header.from=shop.example",
        want: Standing::Passed,
    },
    Sample {
        name: "a good signature local policy refused",
        from: "Shop <orders@shop.example>",
        results: "dkim=policy header.d=shop.example",
        want: Standing::Unsure,
    },
    Sample {
        name: "nothing checked",
        from: "Shop <orders@shop.example>",
        results: "none",
        want: Standing::Unsure,
    },
    Sample {
        name: "a From with two addresses",
        from: "a@shop.example, b@other.example",
        results: "dkim=pass header.d=shop.example",
        want: Standing::Unsure,
    },
];

#[test]
fn the_checks_come_to_what_dmarc_says() {
    let receiver = Receiver::Domains(vec!["provider.example".to_owned()]);
    for sample in SAMPLES {
        let raw = format!(
            "Authentication-Results: mx.provider.example; {}\r\nFrom: {}\r\n\
             To: me@provider.example\r\nSubject: s\r\n\r\nbody\r\n",
            sample.results, sample.from
        );
        let results = authentication_results(raw.as_bytes(), &receiver)
            .unwrap_or_else(|| panic!("{}: the provider's field is not read", sample.name));
        assert_eq!(
            standing(&results),
            sample.want,
            "{}: {results:?}",
            sample.name
        );
    }
}
