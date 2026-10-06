//! The mapping, as tables: every view is the right props, every event the right input.

use ds_shell::accounts::hidden::Hidden;
use ds_shell::accounts::model::{
    CopyState, FieldRole, FieldText, Limitation, ProviderPick, ServiceKey, ServiceOffer,
    SignInFault as ShellFault,
};

use porter_core::sheet::{
    Entry, FieldKind, FieldProblem, FieldSpec, FieldValue, Presence, ProblemKind, ProviderRow,
    Review, ReviewView, RowKind, ServiceChoice, ServiceRow, ServiceState, SheetInput, SheetView,
    SignInFault, SignInView, UserCode,
};
use porter_core::{
    AbsentReason, AccountLabel, CapabilityKind, EndpointUrl, LimitReason, ProviderId, SecretText,
    Toggle, WebUrl,
};

use super::map::{
    Action, Awaiting, ListKey, Out, Sheet, Step, acted, fault_of, kind_of, list_key, mark_of,
    provider_label, role_of, service_key, shown, step_of, typed,
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
