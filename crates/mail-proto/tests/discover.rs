//! Account discovery's pure half: reading configuration documents, and choosing from what
//! documents, SRV records and MX records say. Every document here is synthetic.

use chrono::{DateTime, Utc};
use mail_domain::{AuthPlan, Incoming, OAuthIssuer, Outgoing, Tls, Username};
use mail_proto::discover::autoconfig::{
    self, AuthMethod, AutoconfigError, ServerProtocol, SocketType,
};
use mail_proto::discover::{
    self, MxLead, MxRecord, Side, Source, SrvRecord, Unusable, from_autoconfig, from_mx, from_srv,
};

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

const ADDRESS: &str = "someone@example.test";

/// A document with the given servers inside one provider.
fn doc(servers: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<clientConfig version="1.1">
  <emailProvider id="example.test">
    <domain>example.test</domain>
    <displayName>Example</displayName>
    {servers}
  </emailProvider>
</clientConfig>"#
    )
}

fn server(kind: &str, host: &str, port: u16, socket: &str, user: &str, auth: &[&str]) -> String {
    let auth: String = auth
        .iter()
        .map(|a| format!("<authentication>{a}</authentication>"))
        .collect();
    let tag = if kind == "smtp" {
        "outgoingServer"
    } else {
        "incomingServer"
    };
    format!(
        "<{tag} type=\"{kind}\">
           <hostname>{host}</hostname>
           <port>{port}</port>
           <socketType>{socket}</socketType>
           <username>{user}</username>
           {auth}
         </{tag}>"
    )
}

fn choose(xml: &str) -> Result<mail_domain::presets::Preset, Unusable> {
    from_autoconfig(&autoconfig::parse(xml).expect("parses"), ADDRESS, now())
}

mod parsing {
    use super::*;

    #[test]
    fn every_server_is_read_in_document_order() {
        let xml = doc(&[
            server(
                "imap",
                "imap.example.test",
                993,
                "SSL",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
            server(
                "imap",
                "imap.example.test",
                143,
                "STARTTLS",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
            server(
                "pop3",
                "pop.example.test",
                995,
                "SSL",
                "%EMAILLOCALPART%",
                &["password-encrypted"],
            ),
            server(
                "smtp",
                "smtp.example.test",
                465,
                "SSL",
                "%EMAILADDRESS%",
                &["OAuth2", "password-cleartext"],
            ),
        ]
        .concat());
        let config = autoconfig::parse(&xml).unwrap();
        assert_eq!(config.incoming.len(), 3);
        assert_eq!(config.outgoing.len(), 1);
        assert_eq!(config.incoming[0].protocol, ServerProtocol::Imap);
        assert_eq!(config.incoming[0].socket, SocketType::Ssl);
        assert_eq!(config.incoming[1].socket, SocketType::StartTls);
        assert_eq!(config.incoming[2].protocol, ServerProtocol::Pop3);
        assert_eq!(config.incoming[2].username, "%EMAILLOCALPART%");
        assert_eq!(config.incoming[2].auth, vec![AuthMethod::PasswordEncrypted]);
        assert_eq!(
            config.outgoing[0].auth,
            vec![AuthMethod::OAuth2, AuthMethod::PasswordCleartext]
        );
        assert_eq!(config.oauth_issuer, None);
    }

    #[test]
    fn the_oauth_issuer_is_read_from_its_own_element() {
        let xml = doc(&server(
            "imap",
            "imap.example.test",
            993,
            "SSL",
            "",
            &["OAuth2"],
        ))
        .replace(
            "</clientConfig>",
            "<oAuth2><issuer>accounts.google.com</issuer><scope>x</scope></oAuth2></clientConfig>",
        );
        let config = autoconfig::parse(&xml).unwrap();
        assert_eq!(config.oauth_issuer.as_deref(), Some("accounts.google.com"));
    }

    #[test]
    fn a_server_with_no_host_or_a_bad_port_is_dropped_alone() {
        let xml = doc(&[
            server("imap", "", 993, "SSL", "", &[]),
            "<incomingServer type=\"imap\"><hostname>x.example.test</hostname><port>ninety</port></incomingServer>".to_owned(),
            "<incomingServer type=\"imap\"><hostname>y.example.test</hostname><port>0</port></incomingServer>".to_owned(),
            server("imap", "good.example.test", 993, "SSL", "", &[]),
        ]
        .concat());
        let config = autoconfig::parse(&xml).unwrap();
        assert_eq!(config.incoming.len(), 1);
        assert_eq!(config.incoming[0].hostname, "good.example.test");
    }

    #[test]
    fn a_missing_socket_type_means_no_tls() {
        let xml = doc(
            "<incomingServer type=\"imap\"><hostname>h.example.test</hostname><port>143</port></incomingServer>",
        );
        let config = autoconfig::parse(&xml).unwrap();
        assert_eq!(config.incoming[0].socket, SocketType::Plain);
    }

    #[test]
    fn malformed_documents_are_refused_with_a_reason() {
        assert!(matches!(
            autoconfig::parse("<clientConfig><emailProvider>"),
            Err(AutoconfigError::Xml(_))
        ));
        assert!(matches!(
            autoconfig::parse("<html><body>Not here</body></html>"),
            Err(AutoconfigError::NotClientConfig)
        ));
        assert!(matches!(
            autoconfig::parse("<clientConfig version=\"1.1\"/>"),
            Err(AutoconfigError::NoProvider)
        ));
        assert!(matches!(
            autoconfig::parse(""),
            Err(AutoconfigError::Xml(_))
        ));
    }

    #[test]
    fn a_document_type_declaration_is_refused_rather_than_expanded() {
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE clientConfig [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;">]>
<clientConfig><emailProvider><displayName>&b;</displayName></emailProvider></clientConfig>"#;
        assert!(matches!(
            autoconfig::parse(xml),
            Err(AutoconfigError::Xml(_))
        ));
    }
}

mod selection {
    use super::*;

    #[test]
    fn the_first_implicit_tls_imap_and_smtp_servers_are_chosen() {
        let preset = choose(&doc(&[
            server(
                "imap",
                "starttls.example.test",
                143,
                "STARTTLS",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
            server(
                "imap",
                "imap.example.test",
                993,
                "SSL",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
            server(
                "imap",
                "second.example.test",
                993,
                "SSL",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
            server(
                "pop3",
                "pop.example.test",
                995,
                "SSL",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
            server(
                "smtp",
                "submit.example.test",
                587,
                "STARTTLS",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
            server(
                "smtp",
                "smtp.example.test",
                465,
                "SSL",
                "%EMAILADDRESS%",
                &["password-cleartext"],
            ),
        ]
        .concat()))
        .unwrap();
        assert_eq!(
            preset.plan.incoming,
            Incoming::Imap {
                host: "imap.example.test".to_owned(),
                port: 993,
                tls: Tls::Implicit
            }
        );
        assert_eq!(
            preset.plan.outgoing,
            Outgoing::Smtp {
                host: "smtp.example.test".to_owned(),
                port: 465,
                tls: Tls::Implicit
            }
        );
        assert!(matches!(
            preset.plan.auth,
            AuthPlan::Password {
                username: Username::SameAsAddress,
                ..
            }
        ));
    }

    #[test]
    fn pop3_is_used_only_when_no_imap_server_will_do() {
        let preset = choose(&doc(&[
            server(
                "pop3",
                "pop.example.test",
                995,
                "SSL",
                "%EMAILLOCALPART%",
                &["password-cleartext"],
            ),
            server("imap", "imap.example.test", 143, "STARTTLS", "", &[]),
            server("smtp", "smtp.example.test", 465, "SSL", "", &[]),
        ]
        .concat()))
        .unwrap();
        assert!(matches!(
            preset.plan.incoming,
            Incoming::Pop3 { ref host, port: 995, tls: Tls::Implicit, .. } if host == "pop.example.test"
        ));
        assert!(matches!(
            preset.plan.auth,
            AuthPlan::Password {
                username: Username::LocalPart,
                ..
            }
        ));
    }

    #[test]
    fn starttls_only_is_skipped_and_said() {
        let err = choose(&doc(&[
            server("imap", "imap.example.test", 143, "STARTTLS", "", &[]),
            server("smtp", "smtp.example.test", 465, "SSL", "", &[]),
        ]
        .concat()))
        .unwrap_err();
        let Unusable::NoImplicitTls { side, offered } = &err else {
            panic!("{err:?}")
        };
        assert_eq!(*side, Side::Incoming);
        assert_eq!(offered[0].port, 143);
        assert!(err.to_string().contains("STARTTLS"), "{err}");

        let err = choose(&doc(&[
            server("imap", "imap.example.test", 993, "SSL", "", &[]),
            server("smtp", "smtp.example.test", 587, "STARTTLS", "", &[]),
            server("smtp", "smtp.example.test", 25, "plain", "", &[]),
        ]
        .concat()))
        .unwrap_err();
        assert!(
            matches!(
                err,
                Unusable::NoImplicitTls {
                    side: Side::Outgoing,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn a_side_with_no_server_at_all_is_missing() {
        let err = choose(&doc(&server("imap", "i.example.test", 993, "SSL", "", &[]))).unwrap_err();
        assert_eq!(err, Unusable::Missing(Side::Outgoing));
    }

    #[test]
    fn a_challenge_response_only_server_is_not_one_this_can_sign_in_to() {
        let err = choose(&doc(&[
            server(
                "imap",
                "i.example.test",
                993,
                "SSL",
                "",
                &["password-encrypted"],
            ),
            server("smtp", "s.example.test", 465, "SSL", "", &[]),
        ]
        .concat()))
        .unwrap_err();
        assert!(
            matches!(
                err,
                Unusable::Mechanisms {
                    side: Side::Incoming,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn oauth2_through_a_known_issuer_uses_that_providers_preset() {
        let named = doc(&[
            server("imap", "imap.example.test", 993, "SSL", "", &["OAuth2"]),
            server("smtp", "smtp.example.test", 465, "SSL", "", &["OAuth2"]),
        ]
        .concat())
        .replace(
            "</clientConfig>",
            "<oAuth2><issuer>login.microsoftonline.com</issuer></oAuth2></clientConfig>",
        );
        assert!(matches!(
            choose(&named).unwrap().plan.auth,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Microsoft,
                ..
            }
        ));
        // No issuer element, but the server is one whose tokens come from a known issuer.
        let implied = doc(&[
            server(
                "imap",
                "imap.gmail.com",
                993,
                "SSL",
                "",
                &["OAuth2", "password-cleartext"],
            ),
            server(
                "smtp",
                "smtp.gmail.com",
                465,
                "SSL",
                "",
                &["OAuth2", "password-cleartext"],
            ),
        ]
        .concat());
        let preset = choose(&implied).unwrap();
        assert!(matches!(
            preset.plan.auth,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                ..
            }
        ));
        assert_eq!(preset.plan.address, ADDRESS);
    }

    #[test]
    fn oauth2_through_an_unknown_issuer_falls_back_to_a_password() {
        let xml = doc(&[
            server(
                "imap",
                "imap.example.test",
                993,
                "SSL",
                "",
                &["OAuth2", "password-cleartext"],
            ),
            server(
                "smtp",
                "smtp.example.test",
                465,
                "SSL",
                "",
                &["OAuth2", "password-cleartext"],
            ),
        ]
        .concat())
        .replace(
            "</clientConfig>",
            "<oAuth2><issuer>auth.example.test</issuer></oAuth2></clientConfig>",
        );
        assert!(matches!(
            choose(&xml).unwrap().plan.auth,
            AuthPlan::Password { .. }
        ));
    }

    #[test]
    fn hostnames_have_their_placeholders_filled() {
        let preset = choose(&doc(&[
            server("imap", "mail.%EMAILDOMAIN%", 993, "SSL", "", &[]),
            server("smtp", "mail.%EMAILDOMAIN%", 465, "SSL", "", &[]),
        ]
        .concat()))
        .unwrap();
        assert!(matches!(
            preset.plan.incoming,
            Incoming::Imap { ref host, .. } if host == "mail.example.test"
        ));
    }

    #[test]
    fn a_personal_microsoft_address_is_not_given_the_tenant_preset() {
        let xml = doc(&[
            server("imap", "outlook.office365.com", 993, "SSL", "", &["OAuth2"]),
            server(
                "smtp",
                "smtp.office365.com",
                587,
                "STARTTLS",
                "",
                &["OAuth2"],
            ),
        ]
        .concat());
        let config = autoconfig::parse(&xml).unwrap();
        assert_eq!(
            from_autoconfig(&config, "someone@hotmail.com", now()),
            Err(Unusable::PersonalMicrosoft)
        );
    }
}

/// The username element, as a table: what the document says, what the account logs in as.
#[test]
fn username_placeholders_map_onto_the_existing_forms() {
    let cases: &[(&str, Username)] = &[
        ("%EMAILADDRESS%", Username::SameAsAddress),
        ("", Username::SameAsAddress),
        ("  %EMAILADDRESS%  ", Username::SameAsAddress),
        ("%EMAILLOCALPART%", Username::LocalPart),
        ("%EMAILLOCALPART%@%EMAILDOMAIN%", Username::SameAsAddress),
        (
            "%EMAILDOMAIN%",
            Username::Literal("example.test".to_owned()),
        ),
        (
            "%EMAILLOCALPART%.%EMAILDOMAIN%",
            Username::Literal("someone.example.test".to_owned()),
        ),
        ("fixed-login", Username::Literal("fixed-login".to_owned())),
    ];
    for (raw, want) in cases {
        assert_eq!(&discover::username(raw, ADDRESS), want, "{raw:?}");
    }
}

fn srv(priority: u16, weight: u16, port: u16, target: &str) -> SrvRecord {
    SrvRecord {
        priority,
        weight,
        port,
        target: target.to_owned(),
    }
}

mod srv_records {
    use super::*;

    #[test]
    fn the_lowest_priority_then_heaviest_target_is_chosen() {
        let imaps = [
            srv(20, 0, 993, "backup.example.test."),
            srv(10, 5, 993, "light.example.test."),
            srv(10, 50, 9993, "heavy.example.test."),
        ];
        let submissions = [srv(0, 0, 465, "smtp.example.test.")];
        let preset = from_srv(ADDRESS, &imaps, &[], &submissions, now()).unwrap();
        assert_eq!(
            preset.plan.incoming,
            Incoming::Imap {
                host: "heavy.example.test".to_owned(),
                port: 9993,
                tls: Tls::Implicit
            }
        );
        assert_eq!(
            preset.plan.outgoing,
            Outgoing::Smtp {
                host: "smtp.example.test".to_owned(),
                port: 465,
                tls: Tls::Implicit
            }
        );
    }

    #[test]
    fn pop3s_serves_when_imaps_is_absent_or_refused() {
        let submissions = [srv(0, 0, 465, "smtp.example.test")];
        let pop3s = [srv(0, 0, 995, "pop.example.test")];
        // A target of "." says the service is decidedly not offered.
        let preset = from_srv(ADDRESS, &[srv(0, 0, 0, ".")], &pop3s, &submissions, now()).unwrap();
        assert!(matches!(
            preset.plan.incoming,
            Incoming::Pop3 { port: 995, .. }
        ));
    }

    #[test]
    fn no_submission_record_is_no_configuration() {
        let imaps = [srv(0, 0, 993, "imap.example.test")];
        assert_eq!(
            from_srv(ADDRESS, &imaps, &[], &[], now()),
            Err(Unusable::Missing(Side::Outgoing))
        );
        assert_eq!(
            from_srv(
                ADDRESS,
                &[],
                &[],
                &[srv(0, 0, 465, "s.example.test")],
                now()
            ),
            Err(Unusable::Missing(Side::Incoming))
        );
    }
}

mod mx_records {
    use super::*;

    /// A stand-in for the public-suffix lookup: the last two labels.
    fn registered(host: &str) -> Option<String> {
        let labels: Vec<&str> = host.split('.').collect();
        (labels.len() >= 2).then(|| labels[labels.len() - 2..].join("."))
    }

    fn mx(preference: u16, exchange: &str) -> MxRecord {
        MxRecord {
            preference,
            exchange: exchange.to_owned(),
        }
    }

    #[test]
    fn an_mx_at_google_is_the_gmail_preset() {
        let records = [
            mx(10, "alt1.aspmx.l.google.com."),
            mx(1, "aspmx.l.google.com."),
        ];
        let MxLead::Known(found) = from_mx(ADDRESS, &records, registered, now()).unwrap() else {
            panic!("not recognised")
        };
        assert_eq!(
            found.source,
            Source::Mx {
                exchanger: "aspmx.l.google.com".to_owned(),
                provider: "google.com".to_owned()
            }
        );
        assert!(matches!(
            found.preset.plan.auth,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                ..
            }
        ));
        assert_eq!(found.preset.plan.address, ADDRESS);
        assert!(found.source.to_string().starts_with("via MX → google.com"));
    }

    #[test]
    fn an_mx_at_a_tenant_host_is_the_microsoft_preset() {
        let records = [mx(0, "example-test.mail.protection.outlook.com")];
        let MxLead::Known(found) = from_mx(ADDRESS, &records, registered, now()).unwrap() else {
            panic!("not recognised")
        };
        assert!(matches!(
            found.preset.plan.auth,
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Microsoft,
                ..
            }
        ));
    }

    #[test]
    fn an_unknown_hosting_domain_is_asked_of_the_ispdb() {
        let records = [mx(5, "mx1.hosting.example.")];
        assert_eq!(
            from_mx(ADDRESS, &records, registered, now()).unwrap(),
            MxLead::AskIspdb {
                domain: "hosting.example".to_owned(),
                source: Source::Mx {
                    exchanger: "mx1.hosting.example".to_owned(),
                    provider: "hosting.example".to_owned()
                }
            }
        );
    }

    #[test]
    fn nothing_more_is_learned_from_the_domains_own_mx_or_none() {
        assert_eq!(
            from_mx(ADDRESS, &[mx(0, "mx.example.test")], registered, now()).unwrap(),
            MxLead::Nothing
        );
        assert_eq!(
            from_mx(ADDRESS, &[], registered, now()).unwrap(),
            MxLead::Nothing
        );
        // RFC 7505's null MX: this domain takes no mail.
        assert_eq!(
            from_mx(ADDRESS, &[mx(0, ".")], registered, now()).unwrap(),
            MxLead::Nothing
        );
    }

    #[test]
    fn a_personal_microsoft_address_found_through_mx_says_why_not() {
        let records = [mx(0, "hotmail-com.olc.protection.outlook.com")];
        assert_eq!(
            from_mx("someone@hotmail.com", &records, registered, now()),
            Err(Unusable::PersonalMicrosoft)
        );
    }
}

#[test]
fn every_source_names_itself_the_way_the_confirmation_prints_it() {
    assert_eq!(
        Source::DomainAutoconfig.to_string(),
        "from the domain's autoconfig"
    );
    assert_eq!(Source::Ispdb.to_string(), "from the Thunderbird ISPDB");
    assert_eq!(Source::Srv.to_string(), "from DNS SRV");
}
