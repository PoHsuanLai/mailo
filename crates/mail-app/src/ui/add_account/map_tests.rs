//! The mapping, as tables: every view is the right props, every event the right input.

use ds_shell::accounts::hidden::Hidden;
use ds_shell::accounts::model::{
    FieldRole, FieldText, FormPart, Limitation, ProblemKind as ShellProblemKind, ProviderPick,
    Requirement, ServiceKey, ServiceOffer, SignInFault as ShellFault,
};

use porter_core::sheet::{
    Entry, FieldKind, FieldProblem, FieldSpec, FieldValue, Presence, ProblemKind, Protocol,
    ProviderRow, Review, ReviewView, RowKind, ServiceRow, ServiceState, SheetInput, SheetView,
    SignInFault, SignInView, UserCode, manual_form,
};
use porter_core::{
    AbsentReason, AccountLabel, CapabilityKind, EndpointUrl, LimitReason, ProviderId, Toggle,
    WebUrl,
};

use super::map::{
    Action, Sheet, Step, acted, choice_label, fault_of, kind_of, list_key, mark_of, provider_label,
    role_of, service_key, shown, step_of, typed,
};
use mail_core::account::draft::ListKey;

fn id(text: &str) -> ProviderId {
    ProviderId::parse(text).unwrap()
}

fn rows() -> Vec<ProviderRow> {
    [
        ("google", "Google", "google", RowKind::Provider),
        ("fastmail", "Fastmail", "fastmail", RowKind::Provider),
        (
            "generic-imap",
            "Other email account",
            "mail",
            RowKind::Generic,
        ),
    ]
    .map(|(provider, label, mark, kind)| ProviderRow {
        auth: Default::default(),
        id: id(provider),
        label: label.to_owned(),
        mark: mark.to_owned(),
        kind,
        mark_face: None,
        group: None,
    })
    .to_vec()
}

fn spec(kind: FieldKind, entry: Entry) -> FieldSpec {
    FieldSpec {
        kind,
        entry,
        presence: Presence::Required,
        prefill: None,
    }
}

fn form() -> SheetView {
    SheetView::SignIn(SignInView {
        row: None,
        provider: id("fastmail"),
        fields: vec![
            FieldSpec {
                prefill: Some("ada@example.test".to_owned()),
                ..spec(FieldKind::Address, Entry::Plain)
            },
            spec(FieldKind::Password, Entry::Secret),
        ],
        problem: None,
    })
}

fn review() -> SheetView {
    SheetView::Review(ReviewView {
        row: None,
        provider: id("fastmail"),
        review: Review {
            label: AccountLabel("ada@example.test".to_owned()),
            services: vec![
                ServiceRow {
                    kind: CapabilityKind::Mail,
                    state: ServiceState::Offered(Toggle::On),
                    limit: None,
                },
                ServiceRow {
                    kind: CapabilityKind::Storage,
                    state: ServiceState::Offered(Toggle::Off),
                    limit: Some(LimitReason::AppFolderOnly),
                },
                ServiceRow {
                    kind: CapabilityKind::Notes,
                    state: ServiceState::Absent(AbsentReason::NotOnServer),
                    limit: None,
                },
            ],
            endpoints: vec![],
        },
        allow: None,
        allow_label: None,
    })
}

fn showing(view: SheetView) -> Sheet {
    shown(Sheet::default(), view).0
}

#[test]
fn every_view_is_the_right_step() {
    let url = WebUrl::parse("https://login.example.test/start?x=1").unwrap();
    let page = EndpointUrl::parse("https://login.example.test/device").unwrap();
    let cases: Vec<(&str, SheetView, Option<&str>)> = vec![
        ("providers", SheetView::Providers(rows()), Some("providers")),
        ("sign-in", form(), Some("sign-in")),
        (
            "browser",
            SheetView::BrowserWait {
                row: None,
                provider: id("google"),
                url,
            },
            Some("browser"),
        ),
        (
            "code",
            SheetView::ShowCode {
                row: None,
                provider: id("microsoft"),
                user_code: UserCode("BQKD-4MZP".to_owned()),
                url: page,
            },
            Some("code"),
        ),
        ("review", review(), Some("review")),
        (
            "working",
            SheetView::Working {
                provider: id("fastmail"),
                row: None,
            },
            Some("working"),
        ),
        (
            "failed",
            SheetView::Failed {
                row: None,
                provider: id("fastmail"),
                fault: SignInFault::Unreachable,
            },
            Some("failed"),
        ),
        ("done", SheetView::Done, None),
    ];
    for (name, view, slug) in cases {
        let step = step_of(&showing(view));
        assert_eq!(step.as_ref().map(Step::slug), slug, "{name}");
    }
    assert_eq!(
        step_of(&Sheet::default()),
        None,
        "nothing is shown before the service says"
    );
}

#[test]
fn the_list_leaves_the_generic_row_to_the_lists_own_other() {
    let Some(Step::Providers(props)) = step_of(&showing(SheetView::Providers(rows()))) else {
        panic!("not the list");
    };
    let keys: Vec<&str> = props.providers.iter().map(|p| p.key.0.as_str()).collect();
    assert_eq!(keys, ["google", "fastmail"]);
    let marks: Vec<_> = props.providers.iter().map(|p| p.mark).collect();
    assert_eq!(marks, [mark_of("google"), mark_of("fastmail")]);
}

#[test]
fn the_form_carries_the_prefill_the_typing_and_the_problem() {
    let mut sheet = showing(form());
    let Some(Step::SignIn(props)) = step_of(&sheet) else {
        panic!("not the form");
    };
    assert_eq!(props.provider, "Fastmail");
    assert_eq!(
        props.fields[0].text,
        FieldText::Plain("ada@example.test".to_owned())
    );
    assert_eq!(props.fields[1].text, FieldText::Secret(Hidden::default()));
    assert_eq!(props.problem, None);

    let (next, input) = acted(
        sheet,
        typed(FieldRole::Password, FieldText::Secret(Hidden::new("pw"))),
    );
    sheet = next;
    assert_eq!(input, None, "a keystroke tells the service nothing");
    let Some(Step::SignIn(props)) = step_of(&sheet) else {
        panic!("not the form");
    };
    assert_eq!(props.fields[1].text, FieldText::Secret(Hidden::new("pw")));

    // A refusal marks the field and counts the attempt.
    let (sheet, input) = acted(sheet, Action::Submit);
    assert!(matches!(input, Some(SheetInput::Submit(_))));
    let SheetView::SignIn(view) = sheet.view.clone().unwrap() else {
        unreachable!()
    };
    let refused = SheetView::SignIn(SignInView {
        problem: Some(FieldProblem {
            field: FieldKind::Password,
            problem: ProblemKind::Refused,
        }),
        ..view
    });
    let (sheet, _) = shown(sheet, refused);
    let Some(Step::SignIn(props)) = step_of(&sheet) else {
        panic!("not the form");
    };
    let problem = props.problem.expect("the problem is shown");
    assert_eq!((problem.role, problem.attempt.0), (FieldRole::Password, 1));
    assert_eq!(
        props.fields[0].text,
        FieldText::Plain("ada@example.test".to_owned()),
        "what is not secret stays"
    );
    assert_eq!(
        props.fields[1].text,
        FieldText::Secret(Hidden::default()),
        "the secret is gone"
    );
}

#[test]
fn no_switch_is_drawn_for_a_service_the_add_does_nothing_with() {
    let sheet = showing(review());
    let Some(Step::Review(props)) = step_of(&sheet) else {
        panic!("not the review");
    };
    assert_eq!(props.account, "ada@example.test");
    // Mail and Files are offered, and mailo adds the mail whatever is switched and has no path
    // for Files: neither is a switch. What the provider lacks is still said, with its reason.
    let offers: Vec<_> = props
        .services
        .iter()
        .map(|s| (s.name.as_str(), s.offer, s.limit))
        .collect();
    assert_eq!(
        offers,
        [("Notes", ServiceOffer::Absent(Limitation::NotOnServer), None)]
    );
    assert!(
        props
            .services
            .iter()
            .all(|s| !matches!(s.offer, ServiceOffer::Offered(_))),
        "a switch is drawn that does nothing"
    );
    // Confirm sends the services as the provider offered them, since none can be changed.
    let (_, input) = acted(sheet, Action::Confirm);
    assert!(matches!(
        input,
        Some(SheetInput::Confirm(choices)) if choices.iter().all(|c| c.toggle == Toggle::On || c.kind == CapabilityKind::Storage)
    ));
}

#[test]
fn the_parts_events_map_back_to_porters_values() {
    for (role, kind) in [
        (FieldRole::Address, FieldKind::Address),
        (FieldRole::Server, FieldKind::Server),
        (FieldRole::Username, FieldKind::Username),
        (FieldRole::Password, FieldKind::Password),
        (FieldRole::ApiKey, FieldKind::ApiKey),
        (FieldRole::Token, FieldKind::Token),
        (FieldRole::Protocol, FieldKind::Protocol),
        (FieldRole::Port, FieldKind::Port),
        (FieldRole::Security, FieldKind::Security),
        (FieldRole::OutgoingServer, FieldKind::OutgoingServer),
        (FieldRole::OutgoingPort, FieldKind::OutgoingPort),
        (FieldRole::OutgoingSecurity, FieldKind::OutgoingSecurity),
        (FieldRole::SessionUrl, FieldKind::SessionUrl),
    ] {
        assert_eq!(role_of(kind), role);
        assert_eq!(kind_of(role), kind);
    }
    assert_eq!(list_key(&ProviderPick::Other), Some(ListKey::Other));
    assert_eq!(
        list_key(&ProviderPick::Provider(
            ds_shell::accounts::model::ProviderKey("fastmail".to_owned())
        )),
        Some(ListKey::Provider(id("fastmail")))
    );
    assert_eq!(
        list_key(&ProviderPick::Provider(
            ds_shell::accounts::model::ProviderKey("Not An Id".to_owned())
        )),
        None
    );
    let key = ServiceKey(service_key(CapabilityKind::Mail));
    assert_eq!(key.0, "mail");
    assert!(matches!(
        typed(FieldRole::Password, FieldText::Secret(Hidden::new("x"))),
        Action::Type(FieldKind::Password, FieldValue::Secret(_))
    ));
}

#[test]
fn every_fault_has_its_sentence_and_every_provider_its_words() {
    for (fault, want) in [
        (SignInFault::Refused, ShellFault::Refused),
        (SignInFault::Unreachable, ShellFault::Unreachable),
        (SignInFault::Unreadable, ShellFault::Unreadable),
        (SignInFault::NeedsClientId, ShellFault::NeedsClientId),
        (SignInFault::TimedOut, ShellFault::TimedOut),
        (SignInFault::Cancelled, ShellFault::Cancelled),
        (SignInFault::Forbidden, ShellFault::Forbidden),
        (SignInFault::StoreFailed, ShellFault::StoreFailed),
        (SignInFault::AlreadyAdded, ShellFault::AlreadyAdded),
    ] {
        assert_eq!(fault_of(fault), want);
    }
    for (provider, label) in [
        ("google", "Google"),
        ("microsoft", "Microsoft"),
        ("yahoo", "Yahoo Mail"),
        ("generic-imap", "your email server"),
        ("something-new", "something-new"),
    ] {
        assert_eq!(provider_label(&id(provider)), label);
    }
}

// ---------------------------------------------------------------------------------------------
// The server form typed by hand.

/// The form porter asks when the lookup found no server: IMAP, guessed from `example.test`.
fn servers() -> SheetView {
    SheetView::SignIn(SignInView {
        row: None,
        provider: id("generic-imap"),
        fields: manual_form(Protocol::Imap, Some("example.test")),
        problem: None,
    })
}

fn props(sheet: &Sheet) -> super::map::SignInProps {
    let Some(Step::SignIn(props)) = step_of(sheet) else {
        panic!("not the form");
    };
    props
}

fn type_in(sheet: Sheet, role: FieldRole, text: &str) -> (Sheet, Option<SheetInput>) {
    acted(sheet, typed(role, FieldText::Plain(text.to_owned())))
}

#[test]
fn the_server_form_is_drawn_in_parts_with_its_choices_and_the_usual_ports_as_hints() {
    // The first form is one group: no parts, no choices.
    let first = props(&showing(form()));
    assert!(
        first
            .fields
            .iter()
            .all(|f| f.part.is_none() && f.choices.is_none())
    );
    let form = props(&showing(servers()));
    assert_eq!(
        form.fields.iter().map(|f| f.role).collect::<Vec<_>>(),
        [
            FieldRole::Protocol,
            FieldRole::Server,
            FieldRole::Security,
            FieldRole::Port,
            FieldRole::OutgoingServer,
            FieldRole::OutgoingSecurity,
            FieldRole::OutgoingPort,
            FieldRole::Username,
        ]
    );
    let parts: Vec<_> = form.fields.iter().map(|f| f.part).collect();
    let (incoming, outgoing, sign_in) = (
        Some(FormPart::Incoming),
        Some(FormPart::Outgoing),
        Some(FormPart::SignIn),
    );
    assert_eq!(
        parts,
        [
            incoming, incoming, incoming, incoming, outgoing, outgoing, outgoing, sign_in
        ]
    );
    // Choices are the three fields porter says are choices, in mailo's words.
    let choices = |role: FieldRole| -> Option<Vec<(String, String)>> {
        form.fields
            .iter()
            .find(|f| f.role == role)?
            .choices
            .as_ref()
            .map(|all| {
                all.iter()
                    .map(|c| (c.slug.clone(), c.label.clone()))
                    .collect()
            })
    };
    let pair = |slug: &str, label: &str| (slug.to_owned(), label.to_owned());
    assert_eq!(
        choices(FieldRole::Protocol),
        Some(vec![
            pair("imap", "IMAP"),
            pair("pop3", "POP3"),
            pair("jmap", "JMAP")
        ])
    );
    for role in [FieldRole::Security, FieldRole::OutgoingSecurity] {
        assert_eq!(
            choices(role),
            Some(vec![
                pair("tls", "Secure from the start (SSL/TLS)"),
                pair("starttls", "Secure after connecting (STARTTLS)")
            ])
        );
    }
    for role in [FieldRole::Server, FieldRole::Port, FieldRole::Username] {
        assert_eq!(choices(role), None, "{role:?} is typed");
    }
    // Ports are optional with the usual number as the hint; the rest follow porter's presence.
    let field = |role: FieldRole| form.fields.iter().find(|f| f.role == role).unwrap();
    for (role, hint) in [(FieldRole::Port, "993"), (FieldRole::OutgoingPort, "465")] {
        assert_eq!(field(role).requirement, Requirement::Optional);
        assert_eq!(field(role).hint.as_deref(), Some(hint));
    }
    assert_eq!(field(FieldRole::Server).requirement, Requirement::Required);
    assert_eq!(
        field(FieldRole::Server).text,
        FieldText::Plain("imap.example.test".to_owned()),
        "the guess is what porter prefilled"
    );
    assert_eq!(
        field(FieldRole::Protocol).text,
        FieldText::Plain("imap".to_owned())
    );
}

#[test]
fn a_choice_arrives_as_exactly_its_slug_and_goes_back_as_the_same_slug() {
    for (kind, role) in [
        (FieldKind::Protocol, FieldRole::Protocol),
        (FieldKind::Security, FieldRole::Security),
        (FieldKind::OutgoingSecurity, FieldRole::OutgoingSecurity),
    ] {
        let form = props(&showing(servers()));
        let choices = form
            .fields
            .iter()
            .find(|f| f.role == role)
            .and_then(|f| f.choices.clone())
            .unwrap();
        // Each option is one of porter's slugs, all of them, in porter's order.
        let slugs: Vec<&str> = choices.iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(slugs, kind.choices());
        for choice in &choices {
            assert_eq!(choice.label, choice_label(&choice.slug));
            assert_eq!(
                typed(role, FieldText::Plain(choice.slug.clone())),
                Action::Type(kind, FieldValue::Plain(choice.slug.clone())),
                "a pick is its slug"
            );
        }
    }
}

/// What the form is marked with once Continue is pressed, and whether it went: the service's
/// rules are core's (`account::draft`), and the marks are drawn on the parts' roles.
#[test]
fn a_form_is_marked_and_sent_as_porter_says() {
    use FieldRole::{Port, Protocol, Server, SessionUrl};
    use ShellProblemKind::Invalid;
    type Case = (
        &'static [(FieldRole, &'static str)],
        Option<(FieldRole, ShellProblemKind)>,
        bool,
    );
    const CASES: &[Case] = &[
        (&[(Port, "993")], None, true),
        (&[(Port, "99999")], Some((Port, Invalid)), false),
        (&[(Port, "99999"), (Port, "993")], None, true),
        (
            &[
                (Protocol, "jmap"),
                (SessionUrl, "http://jmap.example.test/session"),
            ],
            Some((SessionUrl, Invalid)),
            false,
        ),
        (
            &[
                (Protocol, "jmap"),
                (SessionUrl, "https://jmap.example.test/session"),
            ],
            None,
            true,
        ),
        // A required field left empty is not sent either, and is not an `Invalid` mark.
        (&[(Server, "")], None, false),
        (&[(Protocol, "pop3")], None, true),
    ];
    for (typed, marked, sent) in CASES {
        let mut sheet = showing(servers());
        for (role, text) in *typed {
            sheet = type_in(sheet, *role, text).0;
        }
        let (sheet, input) = acted(sheet, Action::Submit);
        let mark = props(&sheet)
            .problem
            .map(|problem| (problem.role, problem.kind));
        assert_eq!(mark, *marked, "{typed:?}");
        assert_eq!(
            matches!(input, Some(SheetInput::Submit(_))),
            *sent,
            "{typed:?}: {input:?}"
        );
    }
}

#[test]
fn what_the_service_says_is_wrong_with_the_form_is_marked_as_it_says() {
    let sheet = showing(servers());
    let SheetView::SignIn(view) = sheet.view.clone().unwrap() else {
        unreachable!()
    };
    let said = SheetView::SignIn(SignInView {
        problem: Some(FieldProblem {
            field: FieldKind::OutgoingPort,
            problem: ProblemKind::Invalid,
        }),
        ..view
    });
    let (sheet, _) = shown(sheet, said);
    let problem = props(&sheet).problem.unwrap();
    assert_eq!(
        (problem.role, problem.kind),
        (FieldRole::OutgoingPort, ShellProblemKind::Invalid)
    );
}
