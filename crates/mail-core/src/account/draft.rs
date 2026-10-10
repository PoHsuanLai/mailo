//! The add-account sheet's draft: what the person has typed or switched on the step on screen,
//! and the conversation between that and porter's `SheetView` / `SheetInput`.
//!
//! The sheet is controlled: the service says what to show (a [`SheetView`]) and the window owns
//! every value a step shows (what is typed, the cursor, the switches) in a [`Draft`]. Everything
//! here is a function of a view, a draft and an [`Action`]: [`shown`] takes the view the service
//! sent, [`acted`] takes what the person did and says what, if anything, the service is told.
//! Nothing here draws, reads a secret store or talks to a service; how a step is drawn is the
//! front-end's.
//!
//! A typed password lives in the draft and nowhere else, as porter's `SecretText` (its `Debug`
//! prints nothing); the draft empties it as it sends.

use porter_core::sheet::{
    Entry, FieldAnswer, FieldKind, FieldProblem, FieldSpec, FieldValue, Presence, ProviderRow,
    RowKind, ServiceChoice, ServiceState, SheetInput, SheetView, form_problem, refit,
};
use porter_core::{ProviderId, SecretText, WebUrl};

/// A row of the provider list: a provider, or the generic "Other…" row quire's list adds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ListKey {
    /// A provider of the list.
    Provider(ProviderId),
    /// "Other…": any server that speaks the open protocols.
    Other,
}

/// How many times a form has been sent: a refused field shakes once for each new count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Submits(pub u32);

/// Whether the link or code on screen has been copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopyMark {
    #[default]
    Idle,
    Copied,
}

/// One field of the form, with what it holds: the variant of `value` follows `entry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDraft {
    pub kind: FieldKind,
    pub entry: Entry,
    pub presence: Presence,
    pub value: FieldValue,
}

impl FieldDraft {
    fn of(spec: &FieldSpec, kept: Option<&FieldDraft>) -> FieldDraft {
        let value = match (spec.entry, kept.map(|field| &field.value)) {
            (Entry::Plain, Some(FieldValue::Plain(text))) => FieldValue::Plain(text.clone()),
            (Entry::Plain, _) => FieldValue::Plain(spec.prefill.clone().unwrap_or_default()),
            // A secret never survives a new view: what was sent is gone, and so is the rest.
            (Entry::Secret, _) => FieldValue::Secret(SecretText::new("")),
        };
        FieldDraft {
            kind: spec.kind,
            entry: spec.entry,
            presence: spec.presence,
            value,
        }
    }

    /// The field as a refit form shapes it: what it prefills, except a secret, which stays what
    /// was typed (porter's form has none to carry).
    fn refitted(spec: &FieldSpec, kept: Option<&FieldDraft>) -> FieldDraft {
        let value = match (spec.entry, kept.map(|field| &field.value)) {
            (Entry::Secret, Some(secret @ FieldValue::Secret(_))) => secret.clone(),
            (Entry::Secret, _) => FieldValue::Secret(SecretText::new("")),
            (Entry::Plain, _) => FieldValue::Plain(spec.prefill.clone().unwrap_or_default()),
        };
        FieldDraft {
            kind: spec.kind,
            entry: spec.entry,
            presence: spec.presence,
            value,
        }
    }

    fn answer(&self) -> FieldAnswer {
        FieldAnswer {
            kind: self.kind,
            value: match &self.value {
                FieldValue::Plain(text) => FieldValue::Plain(text.trim().to_owned()),
                secret @ FieldValue::Secret(_) => secret.clone(),
            },
        }
    }
}

/// What the person has typed or switched on the step on screen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    pub query: String,
    pub cursor: Option<ListKey>,
    pub fields: Vec<FieldDraft>,
    pub submits: Submits,
    pub choices: Vec<ServiceChoice>,
    pub copied: CopyMark,
}

/// Whether an answer has gone that the service has not yet followed with a new view. One answer
/// goes per view: a double press (or Return and a click) is one answer, not the next step's too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Awaiting {
    #[default]
    Nothing,
    View,
}

/// The sheet on screen: what the service last said to draw, and what the person has done on it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sheet {
    pub view: Option<SheetView>,
    pub draft: Draft,
    pub awaiting: Awaiting,
}

/// What the person does, as the drawn parts report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Query(String),
    Cursor(ListKey),
    Pick(ListKey),
    /// A form field changed. Secret text arrives as `FieldValue::Secret`, whose `Debug` hides it.
    Type(FieldKind, FieldValue),
    Submit,
    OpenAgain,
    Confirm,
    Back,
    Retry,
    /// Cancel or Escape.
    Cancel,
    /// The window copied the link or code on screen.
    Copied,
}

/// What the window must do besides draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// Open this page in the person's browser: the sheet was shown a browser step, and shown the
    /// same page again (the machine's answer to "Open Again") it is opened again. The service
    /// opens none; the host opens a page it is shown.
    OpenPage(WebUrl),
}

/// `sheet` once the service shows `view`, and what the window must do about it.
pub fn shown(sheet: Sheet, view: SheetView) -> (Sheet, Vec<Out>) {
    let out = match &view {
        SheetView::BrowserWait { url, .. } => vec![Out::OpenPage(url.clone())],
        _ => Vec::new(),
    };
    let draft = match &sheet.view {
        Some(before) => drafted(&sheet.draft, before, &view),
        None => fresh(&view),
    };
    (
        Sheet {
            view: Some(view),
            draft,
            awaiting: Awaiting::Nothing,
        },
        out,
    )
}

/// The draft for `view`, from the draft `old` had for the view before it. The same view again
/// keeps everything; a new view of the same step (the form again after a refusal) keeps what a
/// person would not retype; any other view starts clean.
fn drafted(old: &Draft, before: &SheetView, view: &SheetView) -> Draft {
    if before == view {
        return old.clone();
    }
    match (before, view) {
        (SheetView::Providers(_), SheetView::Providers(rows)) => Draft {
            cursor: old.cursor.clone().filter(|key| {
                rows.iter()
                    .any(|row| key == &ListKey::Provider(row.id.clone()))
            }),
            query: old.query.clone(),
            ..Draft::default()
        },
        (SheetView::SignIn(was), SheetView::SignIn(now)) if was.provider == now.provider => Draft {
            fields: fields_of(&now.fields, &old.fields),
            submits: old.submits,
            ..Draft::default()
        },
        _ => fresh(view),
    }
}

/// The draft of a view that follows a different step, or the first.
fn fresh(view: &SheetView) -> Draft {
    match view {
        SheetView::SignIn(form) => Draft {
            fields: fields_of(&form.fields, &[]),
            ..Draft::default()
        },
        SheetView::Review(review) => Draft {
            choices: review
                .review
                .services
                .iter()
                .filter_map(|row| match row.state {
                    ServiceState::Offered(toggle) => Some(ServiceChoice {
                        kind: row.kind,
                        toggle,
                    }),
                    ServiceState::Absent(_) => None,
                })
                .collect(),
            ..Draft::default()
        },
        _ => Draft::default(),
    }
}

fn fields_of(specs: &[FieldSpec], kept: &[FieldDraft]) -> Vec<FieldDraft> {
    specs
        .iter()
        .map(|spec| FieldDraft::of(spec, kept.iter().find(|field| field.kind == spec.kind)))
        .collect()
}

/// `sheet` after `action`, and the input the service is told, if the action is one. An action that
/// does not belong to the step on screen changes nothing.
pub fn acted(sheet: Sheet, action: Action) -> (Sheet, Option<SheetInput>) {
    let Sheet {
        view,
        draft,
        awaiting,
    } = sheet;
    let (draft, input) = match &view {
        Some(view) => act(view, draft, action),
        None => (draft, None),
    };
    // One answer per view: the second of a double press is dropped. A dismissal always goes.
    let input =
        input.filter(|input| matches!(input, SheetInput::Dismiss) || awaiting == Awaiting::Nothing);
    let awaiting = match &input {
        Some(SheetInput::Dismiss) | None => awaiting,
        Some(_) => Awaiting::View,
    };
    (
        Sheet {
            view,
            draft,
            awaiting,
        },
        input,
    )
}

fn act(view: &SheetView, draft: Draft, action: Action) -> (Draft, Option<SheetInput>) {
    match (view, action) {
        (_, Action::Cancel) => (draft, Some(SheetInput::Dismiss)),
        (SheetView::SignIn(_) | SheetView::Review(_) | SheetView::Failed { .. }, Action::Back) => {
            (draft, Some(SheetInput::Back))
        }
        (SheetView::Failed { .. }, Action::Retry) => (draft, Some(SheetInput::Retry)),
        (SheetView::BrowserWait { .. }, Action::OpenAgain) => (draft, Some(SheetInput::OpenAgain)),
        (SheetView::BrowserWait { .. } | SheetView::ShowCode { .. }, Action::Copied) => (
            Draft {
                copied: CopyMark::Copied,
                ..draft
            },
            None,
        ),
        (SheetView::Providers(rows), action) => providers(rows, draft, action),
        (SheetView::SignIn(form), action) => self::form(&form.fields, draft, action),
        (SheetView::Review(_), Action::Confirm) => {
            let choices = draft.choices.clone();
            (draft, Some(SheetInput::Confirm(choices)))
        }
        _ => (draft, None),
    }
}

fn providers(rows: &[ProviderRow], draft: Draft, action: Action) -> (Draft, Option<SheetInput>) {
    match action {
        Action::Query(query) => (Draft { query, ..draft }, None),
        Action::Cursor(key) => (
            Draft {
                cursor: Some(key),
                ..draft
            },
            None,
        ),
        Action::Pick(key) => {
            let picked = match key {
                ListKey::Provider(id) => rows.iter().find(|row| row.id == id).map(|row| &row.id),
                ListKey::Other => rows
                    .iter()
                    .find(|row| row.kind == RowKind::Generic)
                    .map(|row| &row.id),
            }
            .cloned();
            (draft, picked.map(SheetInput::Pick))
        }
        _ => (draft, None),
    }
}

fn form(specs: &[FieldSpec], draft: Draft, action: Action) -> (Draft, Option<SheetInput>) {
    match action {
        Action::Type(kind, value) => {
            let fields = draft
                .fields
                .into_iter()
                .map(|mut field| {
                    // A value of the other sort than the field holds is not a keystroke of it.
                    let same = matches!(
                        (&field.value, &value),
                        (FieldValue::Plain(_), FieldValue::Plain(_))
                            | (FieldValue::Secret(_), FieldValue::Secret(_))
                    );
                    if field.kind == kind && same {
                        field.value = value.clone();
                    }
                    field
                })
                .collect();
            let draft = Draft { fields, ..draft };
            // A pick of the protocol or a security reshapes the form now, as the service would
            // when it was sent.
            match kind.choices().is_empty() {
                true => (draft, None),
                false => (reshaped(specs, draft), None),
            }
        }
        Action::Submit => submit(specs, draft),
        _ => (draft, None),
    }
}

/// Whether `specs` is the form of a server typed by hand.
pub fn is_manual(specs: &[FieldSpec]) -> bool {
    specs.iter().any(|spec| spec.kind == FieldKind::Protocol)
}

fn answers_of(draft: &Draft) -> Vec<FieldAnswer> {
    draft.fields.iter().map(FieldDraft::answer).collect()
}

/// The form `specs` as the draft's answers shape it: porter's `refit`, so the draft and the
/// service agree on which fields there are.
fn shaped(specs: &[FieldSpec], draft: &Draft) -> Vec<FieldSpec> {
    refit(specs, &answers_of(draft))
}

/// `draft` with the fields a refit form has: the ones it lost are gone, the ones it gained
/// start from their prefill, and a port that follows protocol and security follows.
fn reshaped(specs: &[FieldSpec], draft: Draft) -> Draft {
    if !is_manual(specs) {
        return draft;
    }
    let fields = shaped(specs, &draft)
        .iter()
        .map(|spec| FieldDraft::refitted(spec, draft.fields.iter().find(|f| f.kind == spec.kind)))
        .collect();
    Draft { fields, ..draft }
}

/// What is wrong with the form as typed, if anything: porter's `form_problem`, the one rule the
/// service sends by too.
pub fn problem_of(specs: &[FieldSpec], draft: &Draft) -> Option<FieldProblem> {
    form_problem(&shaped(specs, draft), &answers_of(draft))
}

/// Send the form when it has nothing wrong; the secrets leave the draft with it. A form with a
/// field that cannot be right is not sent: that field is marked.
fn submit(specs: &[FieldSpec], draft: Draft) -> (Draft, Option<SheetInput>) {
    if problem_of(specs, &draft).is_some() {
        return (draft, None);
    }
    let answers = answers_of(&draft);
    let fields = draft
        .fields
        .into_iter()
        .map(|mut field| {
            if let FieldValue::Secret(_) = field.value {
                field.value = FieldValue::Secret(SecretText::new(""));
            }
            field
        })
        .collect();
    (
        Draft {
            fields,
            submits: Submits(draft.submits.0 + 1),
            ..draft
        },
        Some(SheetInput::Submit(answers)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use porter_core::sheet::{
        ProblemKind, Protocol, Review, ReviewView, ServiceRow, SignInFault, SignInView, manual_form,
    };
    use porter_core::{AbsentReason, AccountLabel, CapabilityKind, LimitReason, Toggle};

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

    /// The form porter asks when the lookup found no server: IMAP, guessed from `example.test`.
    fn servers() -> SheetView {
        SheetView::SignIn(SignInView {
            row: None,
            provider: id("generic-imap"),
            fields: manual_form(Protocol::Imap, Some("example.test")),
            problem: None,
        })
    }

    fn showing(view: SheetView) -> Sheet {
        shown(Sheet::default(), view).0
    }

    /// What `kind` holds in the draft, as text; a secret is not text.
    fn text_of(sheet: &Sheet, kind: FieldKind) -> String {
        match &sheet
            .draft
            .fields
            .iter()
            .find(|field| field.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} is not asked"))
            .value
        {
            FieldValue::Plain(text) => text.clone(),
            FieldValue::Secret(_) => String::new(),
        }
    }

    fn kinds(sheet: &Sheet) -> Vec<FieldKind> {
        sheet.draft.fields.iter().map(|field| field.kind).collect()
    }

    fn type_in(sheet: Sheet, kind: FieldKind, text: &str) -> (Sheet, Option<SheetInput>) {
        acted(
            sheet,
            Action::Type(kind, FieldValue::Plain(text.to_owned())),
        )
    }

    #[test]
    fn every_event_is_the_right_input() {
        let url = WebUrl::parse("https://login.example.test/start?x=1").unwrap();
        let browser = SheetView::BrowserWait {
            row: None,
            provider: id("google"),
            url,
        };
        let failed = SheetView::Failed {
            row: None,
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
                SheetView::Working {
                    provider: id("fastmail"),
                    row: None,
                },
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
    fn submit_sends_every_field_once_and_empties_the_secret() {
        let sheet = showing(form());
        let (sheet, _) = acted(
            sheet,
            Action::Type(
                FieldKind::Address,
                FieldValue::Plain(" ada@example.test ".to_owned()),
            ),
        );
        // Required fields first: the empty password stops it.
        let (sheet, input) = acted(sheet, Action::Submit);
        assert_eq!(input, None);
        let (sheet, _) = acted(
            sheet,
            Action::Type(
                FieldKind::Password,
                FieldValue::Secret(SecretText::new("pw")),
            ),
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
    fn a_new_view_of_the_same_step_keeps_what_is_not_secret_and_a_refusal_counts_the_attempt() {
        let sheet = showing(form());
        let (sheet, _) = acted(
            sheet,
            Action::Type(
                FieldKind::Password,
                FieldValue::Secret(SecretText::new("pw")),
            ),
        );
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
        assert_eq!(sheet.draft.submits, Submits(1));
        assert_eq!(
            text_of(&sheet, FieldKind::Address),
            "ada@example.test",
            "what is not secret stays"
        );
        assert_eq!(
            sheet.draft.fields[1].value,
            FieldValue::Secret(SecretText::new("")),
            "the secret is gone"
        );
        // A different step starts clean.
        let (sheet, _) = shown(sheet, review());
        assert!(sheet.draft.fields.is_empty());
        assert_eq!(sheet.draft.choices.len(), 2);
    }

    #[test]
    fn a_browser_step_asks_the_window_to_open_its_page_each_time_it_is_shown() {
        let url = WebUrl::parse("https://login.example.test/start?x=1").unwrap();
        let view = SheetView::BrowserWait {
            row: None,
            provider: id("google"),
            url: url.clone(),
        };
        let (sheet, out) = shown(Sheet::default(), view.clone());
        assert_eq!(out, [Out::OpenPage(url.clone())]);
        let (sheet, _) = acted(sheet, Action::Copied);
        assert_eq!(sheet.draft.copied, CopyMark::Copied);
        // "Open Again" answers with the same page shown again: opened again, and the copy mark kept.
        let (sheet, input) = acted(sheet, Action::OpenAgain);
        assert_eq!(input, Some(SheetInput::OpenAgain));
        assert_eq!(sheet.awaiting, Awaiting::View);
        let (sheet, out) = shown(sheet, view);
        assert_eq!(out, [Out::OpenPage(url)]);
        assert_eq!(sheet.awaiting, Awaiting::Nothing);
        assert_eq!(sheet.draft.copied, CopyMark::Copied);
        // A different step starts clean.
        let (sheet, out) = shown(
            sheet,
            SheetView::Working {
                provider: id("google"),
                row: None,
            },
        );
        assert!(out.is_empty());
        assert_eq!(sheet.draft.copied, CopyMark::Idle);
    }

    #[test]
    fn a_protocol_pick_refits_the_form_at_once_jmap_drops_outgoing_and_pop3_moves_the_ports() {
        let sheet = showing(servers());
        assert_eq!(text_of(&sheet, FieldKind::Port), "993");
        assert_eq!(text_of(&sheet, FieldKind::OutgoingPort), "465");

        // IMAP to POP3: the same fields, the ports follow, the guessed host follows.
        let (pop3, input) = type_in(sheet.clone(), FieldKind::Protocol, "pop3");
        assert_eq!(input, None, "a pick is not sent: the form changes here");
        assert_eq!(kinds(&pop3), kinds(&sheet));
        assert_eq!(text_of(&pop3, FieldKind::Port), "995");
        assert_eq!(text_of(&pop3, FieldKind::OutgoingPort), "465");
        assert_eq!(text_of(&pop3, FieldKind::Server), "pop.example.test");

        // To JMAP: no outgoing fields, no port; a session URL and a token instead.
        let (jmap, _) = type_in(pop3, FieldKind::Protocol, "jmap");
        assert_eq!(
            kinds(&jmap),
            [
                FieldKind::Protocol,
                FieldKind::SessionUrl,
                FieldKind::Token,
                FieldKind::Username
            ]
        );

        // And back to IMAP: the outgoing fields return, the ports are the usual ones again.
        let (imap, _) = type_in(jmap, FieldKind::Protocol, "imap");
        assert_eq!(kinds(&imap), kinds(&sheet));
        assert_eq!(text_of(&imap, FieldKind::Port), "993");

        // A port the person typed is theirs and stays through a pick.
        let (typed_port, _) = type_in(sheet, FieldKind::Port, "1993");
        let (picked, _) = type_in(typed_port, FieldKind::Protocol, "pop3");
        assert_eq!(text_of(&picked, FieldKind::Port), "1993");
        assert_eq!(text_of(&picked, FieldKind::OutgoingPort), "465");
    }

    #[test]
    fn a_form_with_a_wrong_port_is_not_sent_and_mending_it_sends_every_field_once() {
        let sheet = showing(servers());
        assert_eq!(problem_of(&specs(&sheet), &sheet.draft), None);

        let (sheet, _) = type_in(sheet, FieldKind::Port, "99999");
        let problem = problem_of(&specs(&sheet), &sheet.draft).expect("the port is marked");
        assert_eq!(
            (problem.field, problem.problem),
            (FieldKind::Port, ProblemKind::Invalid)
        );
        let (sheet, input) = acted(sheet, Action::Submit);
        assert_eq!(input, None, "nothing is sent while a field cannot be right");
        assert_eq!(sheet.awaiting, Awaiting::Nothing);

        let (sheet, _) = type_in(sheet, FieldKind::Port, "993");
        let (sheet, input) = acted(sheet, Action::Submit);
        let Some(SheetInput::Submit(answers)) = input else {
            panic!("{input:?}");
        };
        assert_eq!(answers.len(), 8);
        assert_eq!(sheet.awaiting, Awaiting::View);
    }

    /// The form on screen, as the service sent it.
    fn specs(sheet: &Sheet) -> Vec<FieldSpec> {
        match sheet.view.as_ref() {
            Some(SheetView::SignIn(form)) => form.fields.clone(),
            other => panic!("not the form: {other:?}"),
        }
    }

    #[test]
    fn a_session_url_that_is_not_https_and_a_missing_server_are_not_sent() {
        let (sheet, _) = type_in(showing(servers()), FieldKind::Protocol, "jmap");
        let (sheet, _) = type_in(
            sheet,
            FieldKind::SessionUrl,
            "http://jmap.example.test/session",
        );
        let (sheet, input) = acted(sheet, Action::Submit);
        assert_eq!(input, None);
        let (sheet, _) = type_in(
            sheet,
            FieldKind::SessionUrl,
            "https://jmap.example.test/session",
        );
        let (_, input) = acted(sheet, Action::Submit);
        assert!(matches!(input, Some(SheetInput::Submit(_))), "{input:?}");

        // A required field left empty is not sent either.
        let (sheet, _) = type_in(showing(servers()), FieldKind::Server, "");
        let (_, input) = acted(sheet, Action::Submit);
        assert_eq!(input, None);
    }
}
