//! The mapping, as tables: every view is the right props, every event the right input.

use ds_shell::accounts::hidden::Hidden;
use ds_shell::accounts::model::{
    CopyState, FieldRole, FieldText, FormPart, Limitation, ProblemKind as ShellProblemKind,
    ProviderPick, Requirement, ServiceKey, ServiceOffer, SignInFault as ShellFault,
};

use porter_core::sheet::{
    Entry, FieldKind, FieldProblem, FieldSpec, FieldValue, Presence, ProblemKind, Protocol,
    ProviderRow, Review, ReviewView, RowKind, ServiceChoice, ServiceRow, ServiceState, SheetInput,
    SheetView, SignInFault, SignInView, UserCode, manual_form,
};
use porter_core::{
    AbsentReason, AccountLabel, CapabilityKind, EndpointUrl, LimitReason, ProviderId, SecretText,
    Toggle, WebUrl,
};

use super::map::{
    Action, Awaiting, ListKey, Out, Sheet, Step, acted, choice_label, fault_of, kind_of, list_key,
    mark_of, provider_label, role_of, service_key, shown, step_of, typed,
};

fn id(text: &str) -> ProviderId {
    ProviderId::parse(text).unwrap()
}

fn rows() -> Vec<ProviderRow> {
    [
        ("google-mail", "Google", "google", RowKind::Provider),
        ("fastmail", "Fastmail", "fastmail", RowKind::Provider),
        ("generic-imap", "Email (IMAP)", "mail", RowKind::Generic),
    ]
    .map(|(provider, label, mark, kind)| ProviderRow {
        id: id(provider),
        label: label.to_owned(),
        mark: mark.to_owned(),
        kind,
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
                provider: id("google-mail"),
                url,
            },
            Some("browser"),
        ),
        (
            "code",
            SheetView::ShowCode {
                provider: id("microsoft"),
                user_code: UserCode("BQKD-4MZP".to_owned()),
                url: page,
            },
            Some("code"),
        ),
        ("review", review(), Some("review")),
        (
            "working",
            SheetView::Working(id("fastmail")),
            Some("working"),
        ),
        (
            "failed",
            SheetView::Failed {
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
    assert_eq!(keys, ["google-mail", "fastmail"]);
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
fn submit_sends_every_field_once_and_empties_the_secret() {
    let sheet = showing(form());
    let (sheet, _) = acted(
        sheet,
        typed(
            FieldRole::Address,
            FieldText::Plain(" ada@example.test ".to_owned()),
        ),
    );
    // Required fields first: the empty password stops it.
    let (sheet, input) = acted(sheet, Action::Submit);
    assert_eq!(input, None);
    let (sheet, _) = acted(
        sheet,
        typed(FieldRole::Password, FieldText::Secret(Hidden::new("pw"))),
    );
    let (sheet, input) = acted(sheet, Action::Submit);
    let Some(SheetInput::Submit(answers)) = input else {
        panic!("{input:?}");
    };
    assert_eq!(answers.len(), 2);
    assert_eq!(
        answers[0].value,
        FieldValue::Plain("ada@example.test".to_owned())
    );
    assert_eq!(answers[1].value, FieldValue::Secret(SecretText::new("pw")));
    assert_eq!(sheet.awaiting, Awaiting::View);
    assert!(
        !format!("{sheet:?}").contains("\"pw\""),
        "the draft kept the password"
    );
    // A second press before the service has answered is not a second answer.
    let (_, again) = acted(sheet, Action::Submit);
    assert_eq!(again, None);
}

#[test]
fn every_event_is_the_right_input() {
    let url = WebUrl::parse("https://login.example.test/start?x=1").unwrap();
    let browser = SheetView::BrowserWait {
        provider: id("google-mail"),
        url,
    };
    let failed = SheetView::Failed {
        provider: id("fastmail"),
        fault: SignInFault::Unreachable,
    };
    let cases: Vec<(&str, SheetView, Action, Option<SheetInput>)> = vec![
        (
            "pick",
            SheetView::Providers(rows()),
            Action::Pick(ListKey::Provider(id("fastmail"))),
            Some(SheetInput::Pick(id("fastmail"))),
        ),
        (
            "pick other",
            SheetView::Providers(rows()),
            Action::Pick(ListKey::Other),
            Some(SheetInput::Pick(id("generic-imap"))),
        ),
        (
            "pick unknown",
            SheetView::Providers(rows()),
            Action::Pick(ListKey::Provider(id("nextcloud"))),
            None,
        ),
        (
            "query",
            SheetView::Providers(rows()),
            Action::Query("fast".to_owned()),
            None,
        ),
        (
            "cancel",
            SheetView::Providers(rows()),
            Action::Cancel,
            Some(SheetInput::Dismiss),
        ),
        (
            "cancel working",
            SheetView::Working(id("fastmail")),
            Action::Cancel,
            Some(SheetInput::Dismiss),
        ),
        (
            "back from the form",
            form(),
            Action::Back,
            Some(SheetInput::Back),
        ),
        (
            "back from the list",
            SheetView::Providers(rows()),
            Action::Back,
            None,
        ),
        (
            "back from failed",
            failed.clone(),
            Action::Back,
            Some(SheetInput::Back),
        ),
        (
            "retry",
            failed.clone(),
            Action::Retry,
            Some(SheetInput::Retry),
        ),
        ("retry on the form", form(), Action::Retry, None),
        (
            "open again",
            browser.clone(),
            Action::OpenAgain,
            Some(SheetInput::OpenAgain),
        ),
        ("open again elsewhere", form(), Action::OpenAgain, None),
        ("copied", browser, Action::Copied, None),
        (
            "confirm",
            review(),
            Action::Confirm,
            Some(SheetInput::Confirm(vec![
                ServiceChoice {
                    kind: CapabilityKind::Mail,
                    toggle: Toggle::On,
                },
                ServiceChoice {
                    kind: CapabilityKind::Storage,
                    toggle: Toggle::Off,
                },
            ])),
        ),
        ("confirm on the form", form(), Action::Confirm, None),
    ];
    for (name, view, action, want) in cases {
        let (_, input) = acted(showing(view), action);
        assert_eq!(input, want, "{name}");
    }
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
fn a_browser_step_asks_the_window_to_open_its_page_each_time_it_is_shown() {
    let url = WebUrl::parse("https://login.example.test/start?x=1").unwrap();
    let view = SheetView::BrowserWait {
        provider: id("google-mail"),
        url: url.clone(),
    };
    let (sheet, out) = shown(Sheet::default(), view.clone());
    assert_eq!(out, [Out::OpenPage(url.clone())]);
    let (sheet, _) = acted(sheet, Action::Copied);
    let Some(Step::Browser { copied, .. }) = step_of(&sheet) else {
        panic!("not the browser");
    };
    assert_eq!(copied, CopyState::Copied);
    // "Open Again" answers with the same page shown again: opened again, and the copy mark kept.
    let (sheet, input) = acted(sheet, Action::OpenAgain);
    assert_eq!(input, Some(SheetInput::OpenAgain));
    let (sheet, out) = shown(sheet, view);
    assert_eq!(out, [Out::OpenPage(url)]);
    assert_eq!(sheet.awaiting, Awaiting::Nothing);
    let Some(Step::Browser { copied, .. }) = step_of(&sheet) else {
        panic!("not the browser");
    };
    assert_eq!(copied, CopyState::Copied);
    // A different step starts clean.
    let (sheet, out) = shown(sheet, SheetView::Working(id("google-mail")));
    assert!(out.is_empty());
    assert_eq!(sheet.draft.copied, super::map::CopyMark::Idle);
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
    ] {
        assert_eq!(fault_of(fault), want);
    }
    for (provider, label) in [
        ("google-mail", "Google"),
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

fn roles(sheet: &Sheet) -> Vec<FieldRole> {
    props(sheet).fields.iter().map(|f| f.role).collect()
}

fn text_of(sheet: &Sheet, role: FieldRole) -> String {
    match &props(sheet)
        .fields
        .iter()
        .find(|f| f.role == role)
        .unwrap_or_else(|| panic!("{role:?} is not asked"))
        .text
    {
        FieldText::Plain(text) => text.clone(),
        FieldText::Secret(_) => String::new(),
    }
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
            Some(vec![pair("tls", "SSL/TLS"), pair("starttls", "STARTTLS")])
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

#[test]
fn a_protocol_pick_refits_the_form_at_once_jmap_drops_outgoing_and_pop3_moves_the_ports() {
    let sheet = showing(servers());
    assert_eq!(text_of(&sheet, FieldRole::Port), "993");
    assert_eq!(text_of(&sheet, FieldRole::OutgoingPort), "465");

    // IMAP to POP3: the same fields, the ports follow, the guessed host follows.
    let (pop3, input) = type_in(sheet.clone(), FieldRole::Protocol, "pop3");
    assert_eq!(input, None, "a pick is not sent: the form changes here");
    assert_eq!(roles(&pop3), roles(&sheet));
    assert_eq!(text_of(&pop3, FieldRole::Port), "995");
    assert_eq!(text_of(&pop3, FieldRole::OutgoingPort), "465");
    assert_eq!(text_of(&pop3, FieldRole::Server), "pop.example.test");
    let hint = |sheet: &Sheet, role: FieldRole| {
        props(sheet)
            .fields
            .iter()
            .find(|f| f.role == role)
            .unwrap()
            .hint
            .clone()
    };
    assert_eq!(hint(&pop3, FieldRole::Port).as_deref(), Some("995"));

    // The security is a pick too: STARTTLS moves the ports to theirs.
    let (starttls, _) = type_in(pop3.clone(), FieldRole::Security, "starttls");
    assert_eq!(text_of(&starttls, FieldRole::Port), "110");
    assert_eq!(hint(&starttls, FieldRole::Port).as_deref(), Some("110"));
    let (starttls, _) = type_in(starttls, FieldRole::OutgoingSecurity, "starttls");
    assert_eq!(text_of(&starttls, FieldRole::OutgoingPort), "587");
    assert_eq!(
        hint(&starttls, FieldRole::OutgoingPort).as_deref(),
        Some("587")
    );

    // To JMAP: no outgoing fields, no port; a session URL and a token instead.
    let (jmap, _) = type_in(pop3.clone(), FieldRole::Protocol, "jmap");
    assert_eq!(
        roles(&jmap),
        [
            FieldRole::Protocol,
            FieldRole::SessionUrl,
            FieldRole::Token,
            FieldRole::Username
        ]
    );
    assert_eq!(
        text_of(&jmap, FieldRole::SessionUrl),
        "https://example.test/.well-known/jmap"
    );
    let token = props(&jmap)
        .fields
        .into_iter()
        .find(|f| f.role == FieldRole::Token)
        .unwrap();
    assert!(matches!(token.text, FieldText::Secret(_)));
    assert_eq!(token.requirement, Requirement::Optional);

    // And back to IMAP: the outgoing fields return, the ports are the usual ones again.
    let (imap, _) = type_in(jmap, FieldRole::Protocol, "imap");
    assert_eq!(roles(&imap), roles(&sheet));
    assert_eq!(text_of(&imap, FieldRole::Port), "993");
    assert_eq!(text_of(&imap, FieldRole::Server), "imap.example.test");

    // A port the person typed is theirs and stays through a pick.
    let (typed_port, _) = type_in(sheet, FieldRole::Port, "1993");
    let (picked, _) = type_in(typed_port, FieldRole::Protocol, "pop3");
    assert_eq!(text_of(&picked, FieldRole::Port), "1993");
    assert_eq!(text_of(&picked, FieldRole::OutgoingPort), "465");
}

#[test]
fn a_form_with_a_wrong_port_is_marked_invalid_as_typed_and_is_not_sent() {
    let sheet = showing(servers());
    let (sheet, _) = type_in(sheet, FieldRole::Port, "993");
    assert_eq!(props(&sheet).problem, None, "a good form has no mark");

    // The mark is what quire draws Continue disabled for.
    let (sheet, _) = type_in(sheet, FieldRole::Port, "99999");
    let problem = props(&sheet).problem.expect("the port is marked");
    assert_eq!(
        (problem.role, problem.kind),
        (FieldRole::Port, ShellProblemKind::Invalid)
    );
    let (sheet, input) = acted(sheet, Action::Submit);
    assert_eq!(input, None, "nothing is sent while a field cannot be right");
    assert_eq!(sheet.awaiting, Awaiting::Nothing);

    // Mending it clears the mark and the form goes, with every field once.
    let (sheet, _) = type_in(sheet, FieldRole::Port, "993");
    assert_eq!(props(&sheet).problem, None);
    let (sheet, input) = acted(sheet, Action::Submit);
    let Some(SheetInput::Submit(answers)) = input else {
        panic!("{input:?}");
    };
    assert_eq!(answers.len(), 8);
    assert_eq!(sheet.awaiting, Awaiting::View);
}

#[test]
fn a_session_url_that_is_not_https_and_a_missing_server_are_what_porter_says() {
    let (sheet, _) = type_in(showing(servers()), FieldRole::Protocol, "jmap");
    let (sheet, _) = type_in(
        sheet,
        FieldRole::SessionUrl,
        "http://jmap.example.test/session",
    );
    let (sheet, input) = acted(sheet, Action::Submit);
    assert_eq!(input, None);
    let problem = props(&sheet).problem.unwrap();
    assert_eq!(
        (problem.role, problem.kind),
        (FieldRole::SessionUrl, ShellProblemKind::Invalid)
    );
    let (sheet, _) = type_in(
        sheet,
        FieldRole::SessionUrl,
        "https://jmap.example.test/session",
    );
    let (_, input) = acted(sheet, Action::Submit);
    assert!(matches!(input, Some(SheetInput::Submit(_))), "{input:?}");

    // A required field left empty is not sent either, and is not an `Invalid` mark.
    let (sheet, _) = type_in(showing(servers()), FieldRole::Server, "");
    let (sheet, input) = acted(sheet, Action::Submit);
    assert_eq!(input, None);
    assert_eq!(props(&sheet).problem, None);
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

#[test]
fn a_form_picked_as_pop3_goes_with_every_field_once() {
    let (sheet, _) = type_in(showing(servers()), FieldRole::Protocol, "pop3");
    let (sheet, input) = acted(sheet, Action::Submit);
    let Some(SheetInput::Submit(answers)) = input else {
        panic!("{input:?} {:?}", props(&sheet).problem);
    };
    assert_eq!(answers.len(), 8);
    assert_eq!(
        answers[0].value,
        FieldValue::Plain("pop3".to_owned()),
        "the protocol goes as its slug"
    );
}
