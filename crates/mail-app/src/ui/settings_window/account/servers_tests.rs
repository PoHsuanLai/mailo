use super::{Receiving, Refusal, Row, Server, ServerForm, editable, form_of, plan_from};
use crate::ui::copy::errors;
use mail_domain::{
    AccountPlan, AuthPlan, HttpAuth, Incoming, LeaveOnServer, Outgoing, SaslMech, Tls, Username,
};
use porter_provider::Issuer;

const ADDRESS: &str = "ada@example.com";

fn imap_plan(username: Username) -> AccountPlan {
    AccountPlan {
        address: ADDRESS.to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example.com".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example.com".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    }
}

fn server(host: &str, port: &str, tls: Tls) -> Server {
    Server {
        host: host.to_owned(),
        port: port.to_owned(),
        tls,
    }
}

#[test]
fn an_untouched_form_makes_the_same_plan() {
    for username in [
        Username::SameAsAddress,
        Username::LocalPart,
        Username::Literal("ada.l".to_owned()),
    ] {
        let plan = imap_plan(username);
        assert_eq!(plan_from(&plan, &form_of(&plan)), Ok(plan.clone()));
    }
}

#[test]
fn each_edited_row_lands_in_its_place_in_the_plan() {
    let plan = imap_plan(Username::SameAsAddress);
    let form = ServerForm {
        incoming: Receiving::Server(server(" mail.example.org ", "143", Tls::StartTlsRequired)),
        outgoing: Some(server("out.example.org", "587", Tls::StartTlsRequired)),
        username: Some("ada".to_owned()),
    };
    let edited = plan_from(&plan, &form).unwrap();
    assert_eq!(
        edited.incoming,
        Incoming::Imap {
            host: "mail.example.org".to_owned(),
            port: 143,
            tls: Tls::StartTlsRequired,
        }
    );
    assert_eq!(
        edited.outgoing,
        Outgoing::Smtp {
            host: "out.example.org".to_owned(),
            port: 587,
            tls: Tls::StartTlsRequired,
        }
    );
    assert_eq!(edited.username(), "ada");
    assert_eq!(
        edited.sasl(),
        plan.sasl(),
        "what the form does not show is carried over"
    );
}

#[test]
fn a_pop3_account_keeps_what_it_does_with_downloaded_mail() {
    let mut plan = imap_plan(Username::SameAsAddress);
    plan.incoming = Incoming::Pop3 {
        host: "pop.example.com".to_owned(),
        port: 995,
        tls: Tls::Implicit,
        leave: LeaveOnServer::DeleteAfterFetch,
    };
    let mut form = form_of(&plan);
    form.incoming = Receiving::Server(server("pop2.example.com", "995", Tls::Implicit));
    let edited = plan_from(&plan, &form).unwrap();
    assert!(matches!(
        edited.incoming,
        Incoming::Pop3 { ref host, leave: LeaveOnServer::DeleteAfterFetch, .. } if host == "pop2.example.com"
    ));
}

#[test]
fn a_wrong_row_is_named() {
    let plan = imap_plan(Username::SameAsAddress);
    let cases: [(ServerForm, Refusal); 5] = [
        (
            ServerForm {
                incoming: Receiving::Server(server("  ", "993", Tls::Implicit)),
                ..form_of(&plan)
            },
            Refusal {
                row: Row::Host,
                why: errors::NO_HOST.to_owned(),
            },
        ),
        (
            ServerForm {
                incoming: Receiving::Server(server("imap.example.com", "99999", Tls::Implicit)),
                ..form_of(&plan)
            },
            Refusal {
                row: Row::Port,
                why: errors::BAD_PORT.to_owned(),
            },
        ),
        (
            ServerForm {
                incoming: Receiving::Server(server("imap.example.com", "0", Tls::Implicit)),
                ..form_of(&plan)
            },
            Refusal {
                row: Row::Port,
                why: errors::BAD_PORT.to_owned(),
            },
        ),
        (
            ServerForm {
                outgoing: Some(server("smtp.example.com", "smtp", Tls::Implicit)),
                ..form_of(&plan)
            },
            Refusal {
                row: Row::OutgoingPort,
                why: errors::BAD_PORT.to_owned(),
            },
        ),
        (
            ServerForm {
                username: Some(String::new()),
                ..form_of(&plan)
            },
            Refusal {
                row: Row::Username,
                why: errors::NO_USER_NAME.to_owned(),
            },
        ),
    ];
    for (form, refused) in cases {
        assert_eq!(plan_from(&plan, &form), Err(refused), "{form:?}");
    }
}

#[test]
fn what_a_provider_decided_is_not_offered() {
    let graph = AccountPlan {
        incoming: Incoming::Graph,
        outgoing: Outgoing::Graph,
        auth: AuthPlan::OAuth {
            issuer: Issuer::Microsoft,
            scopes: Vec::new(),
        },
        ..imap_plan(Username::SameAsAddress)
    };
    let form = form_of(&graph);
    assert_eq!(form.incoming, Receiving::Fixed);
    assert_eq!(form.outgoing, None);
    assert_eq!(
        form.username, None,
        "a browser sign-in has no user name to type"
    );
    assert!(!editable(&form));

    let jmap = AccountPlan {
        incoming: Incoming::Jmap {
            session: "https://jmap.example.com/.well-known/jmap".to_owned(),
            auth: HttpAuth::Bearer,
        },
        outgoing: Outgoing::Jmap,
        ..imap_plan(Username::SameAsAddress)
    };
    let mut form = form_of(&jmap);
    assert!(editable(&form));
    form.incoming = Receiving::Session("https://mail.example.org/jmap".to_owned());
    assert!(matches!(
        plan_from(&jmap, &form).unwrap().incoming,
        Incoming::Jmap { ref session, auth: HttpAuth::Bearer } if session == "https://mail.example.org/jmap"
    ));
}
