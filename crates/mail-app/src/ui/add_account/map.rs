//! The add-account window's one mapping: porter's `SheetView` to the props of quire's
//! `ds-shell::accounts` parts, and what the parts report back to the `SheetInput` the service
//! is told. The same mapping sill's accounts sheet makes (`sill-overlays/src/accounts_sheet`), in
//! this window: the sheet is controlled, so the window owns every value a step shows (what is
//! typed, the cursor, the switches) in a [`Draft`], and everything here is a function of a view,
//! a draft and an action. Nothing here draws, reads a secret store or talks to a service.
//!
//! | `SheetView`   | drawn as          | its events                                              |
//! | ------------- | ----------------- | ------------------------------------------------------- |
//! | `Providers`   | `ProviderList`    | `Query`, `Cursor`, `Pick`, `Cancel`                     |
//! | `SignIn`      | `SignInForm`      | `Type`, `Submit`, `Back`, `Cancel`                      |
//! | `BrowserWait` | `BrowserWait`     | `OpenAgain`, `Copied`, `Cancel`                         |
//! | `ShowCode`    | `ShowCode`        | `Copied`, `Cancel`                                      |
//! | `Review`      | `ReviewServices`  | `Confirm`, `Back`, `Cancel`                             |
//! | `Working`     | `SignInWorking`   | `Cancel`                                                |
//! | `Failed`      | `SignInFailed`    | `Retry`, `Back`, `Cancel`                               |
//! | `Done`        | nothing           | (the window closes)                                     |
//! | `Consent`     | nothing           | mailo asks no one's consent: it is its service's only caller |
//!
//! The server form a person types by hand (porter's `manual_form`) is the same `SignIn` view with
//! more fields: a protocol, a server, a security and a port for each direction, the login name,
//! and for JMAP a session URL and a token. Three of them are choices (the protocol and the two
//! securities, `FieldKind::choices()` worded here), a pick refits the form at once
//! (`porter_core::sheet::refit`: JMAP has no outgoing server, a port follows protocol and
//! security until one is typed), and a form `form_problem` finds wrong is not sent: its field is
//! marked as it is typed, which quire draws as Continue disabled.
//!
//! A typed password lives in the draft and nowhere else, as porter's `SecretText` (its `Debug`
//! prints nothing); it reaches the parts as quire's `Hidden` and the service as the one
//! `SheetInput::Submit`, and the draft empties it as it sends.

use ds::components::content::provider_mark::MarkProvider;
use ds_shell::accounts::hidden::Hidden;
use ds_shell::accounts::model::{
    Attempt, Choice, CopyState, FieldProblem as ShellProblem, FieldRole, FieldText, FormField,
    FormPart, Limitation, ProblemKind as ShellProblemKind, ProviderEntry, ProviderKey,
    ProviderPick, Requirement, ServiceKey, ServiceLine, ServiceOffer, SignInFault as ShellFault,
};
use porter_core::sheet::{
    Entry, FieldAnswer, FieldKind, FieldProblem, FieldSpec, FieldValue, Presence, ProblemKind,
    ProviderRow, RowKind, ServiceChoice, ServiceState, SheetInput, SheetView, SignInFault,
    form_problem, refit,
};
use porter_core::{AbsentReason, CapabilityKind, LimitReason, ProviderId, SecretText, WebUrl};

// ---------------------------------------------------------------------------------------------
// What the person has done and not yet sent.

/// A row of the provider list: a provider, or the generic "Other…" row quire's list adds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ListKey {
    /// A provider of the list.
    Provider(ProviderId),
    /// "Other…": any server that speaks the open protocols.
    Other,
}

/// How many times a form has been sent: a refused field shakes once for each new count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Submits(pub u32);

/// Whether the link or code on screen has been copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CopyMark {
    #[default]
    Idle,
    Copied,
}

/// One field of the form, with what it holds: the variant of `value` follows `entry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FieldDraft {
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
pub(super) struct Draft {
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
pub(super) enum Awaiting {
    #[default]
    Nothing,
    View,
}

/// The sheet on screen: what the service last said to draw, and what the person has done on it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Sheet {
    pub view: Option<SheetView>,
    pub draft: Draft,
    pub awaiting: Awaiting,
}

/// What the person does, as the drawn parts report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
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
pub(super) enum Out {
    /// Open this page in the person's browser: the sheet was shown a browser step, and shown the
    /// same page again (the machine's answer to "Open Again") it is opened again. The service
    /// opens none; the host opens a page it is shown.
    OpenPage(WebUrl),
}

/// `sheet` once the service shows `view`, and what the window must do about it.
pub(super) fn shown(sheet: Sheet, view: SheetView) -> (Sheet, Vec<Out>) {
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
pub(super) fn acted(sheet: Sheet, action: Action) -> (Sheet, Option<SheetInput>) {
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
fn is_manual(specs: &[FieldSpec]) -> bool {
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
fn problem_of(specs: &[FieldSpec], draft: &Draft) -> Option<FieldProblem> {
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

// ---------------------------------------------------------------------------------------------
// Props: what each step is given.

/// What the provider list is given.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ProvidersProps {
    pub providers: Vec<ProviderEntry>,
    pub query: String,
    pub cursor: Option<ProviderPick>,
}

/// What the sign-in form is given.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SignInProps {
    pub provider: String,
    pub mark: MarkProvider,
    pub fields: Vec<FormField>,
    pub problem: Option<ShellProblem>,
}

/// What the review is given.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ReviewProps {
    pub account: String,
    pub services: Vec<ServiceLine>,
    pub allow: Option<String>,
}

/// The step on screen, as props.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Step {
    Providers(ProvidersProps),
    SignIn(SignInProps),
    Browser {
        provider: String,
        url: String,
        copied: CopyState,
    },
    Code {
        provider: String,
        code: String,
        url: String,
        copied: CopyState,
    },
    Review(ReviewProps),
    Working {
        provider: String,
    },
    Failed {
        provider: String,
        why: ShellFault,
    },
}

impl Step {
    /// The step's word, for its key and its `data-step`.
    pub(super) fn slug(&self) -> &'static str {
        match self {
            Step::Providers(_) => "providers",
            Step::SignIn(_) => "sign-in",
            Step::Browser { .. } => "browser",
            Step::Code { .. } => "code",
            Step::Review(_) => "review",
            Step::Working { .. } => "working",
            Step::Failed { .. } => "failed",
        }
    }
}

/// The props of `sheet`'s view, or `None` when there is nothing to draw: no view yet, `Done`
/// (the window closes), and a consent (mailo's service asks none).
pub(super) fn step_of(sheet: &Sheet) -> Option<Step> {
    let draft = &sheet.draft;
    Some(match sheet.view.as_ref()? {
        SheetView::Providers(rows) => Step::Providers(ProvidersProps {
            // The generic row is the list's own "Other…".
            providers: rows
                .iter()
                .filter(|row| row.kind == RowKind::Provider)
                .map(|row| ProviderEntry {
                    key: ProviderKey(row.id.as_str().to_owned()),
                    label: row.label.clone(),
                    mark: mark_of(&row.mark),
                })
                .collect(),
            query: draft.query.clone(),
            cursor: draft.cursor.as_ref().map(|key| match key {
                ListKey::Provider(id) => {
                    ProviderPick::Provider(ProviderKey(id.as_str().to_owned()))
                }
                ListKey::Other => ProviderPick::Other,
            }),
        }),
        SheetView::SignIn(form) => {
            let manual = is_manual(&form.fields);
            Step::SignIn(SignInProps {
                provider: provider_label(&form.provider),
                mark: mark_of(form.provider.as_str()),
                fields: draft
                    .fields
                    .iter()
                    .map(|field| form_field(field, draft, manual))
                    .collect(),
                problem: form_marked(&form.fields, form.problem, draft),
            })
        }
        SheetView::BrowserWait { provider, url } => Step::Browser {
            provider: provider_label(provider),
            url: url.as_str().to_owned(),
            copied: copy_state(draft),
        },
        SheetView::ShowCode {
            provider,
            user_code,
            url,
        } => Step::Code {
            provider: provider_label(provider),
            code: user_code.0.clone(),
            url: url.as_str().to_owned(),
            copied: copy_state(draft),
        },
        SheetView::Review(review) => Step::Review(ReviewProps {
            account: review.review.label.0.clone(),
            // A switch is drawn only for a service the add would act on, and mailo acts on none:
            // it adds the account's mail and has no path for any other service of a provider. A
            // service the provider lacks is still said, with its reason, and a switch that would
            // do nothing is not drawn.
            services: review
                .review
                .services
                .iter()
                .filter_map(|row| match row.state {
                    ServiceState::Offered(_) => None,
                    ServiceState::Absent(reason) => Some(ServiceLine {
                        key: ServiceKey(service_key(row.kind)),
                        name: service_name(row.kind).to_owned(),
                        offer: ServiceOffer::Absent(absence_of(reason)),
                        limit: row.limit.map(limit_of),
                    }),
                })
                .collect(),
            allow: None,
        }),
        SheetView::Working(provider) => Step::Working {
            provider: provider_label(provider),
        },
        SheetView::Failed { provider, fault } => Step::Failed {
            provider: provider_label(provider),
            why: fault_of(*fault),
        },
        SheetView::Done | SheetView::Consent(_) => return None,
    })
}

/// One field of the form, as quire's part is given it.
fn form_field(field: &FieldDraft, draft: &Draft, manual: bool) -> FormField {
    let requirement = match field.presence {
        Presence::Required => Requirement::Required,
        Presence::Optional => Requirement::Optional,
    };
    let text = match &field.value {
        FieldValue::Secret(text) => FieldText::Secret(Hidden::new(text.expose())),
        FieldValue::Plain(text) => FieldText::Plain(text.clone()),
    };
    let mut out = FormField::new(role_of(field.kind), requirement, text);
    if !field.kind.choices().is_empty() {
        out = out.choosing(choices_of(field.kind));
    }
    if matches!(field.kind, FieldKind::Port | FieldKind::OutgoingPort) {
        out = out.hinted(usual_port(field.kind, draft).to_string());
    }
    // The server form is long enough to be read in parts; the first form (an address, a
    // password) is one group.
    match manual {
        true => out.in_part(part_of(field.kind)),
        false => out,
    }
}

/// The field to mark: what the service said was wrong, or the first field that cannot be right as
/// typed. An `Invalid` mark is also what quire draws Continue disabled for.
fn form_marked(
    specs: &[FieldSpec],
    said: Option<FieldProblem>,
    draft: &Draft,
) -> Option<ShellProblem> {
    let mark = |problem: FieldProblem, attempt: u32| ShellProblem {
        role: role_of(problem.field),
        kind: match problem.problem {
            ProblemKind::Missing => ShellProblemKind::Missing,
            ProblemKind::Refused => ShellProblemKind::Refused,
            ProblemKind::Invalid => ShellProblemKind::Invalid,
        },
        attempt: Attempt(attempt),
    };
    if let Some(problem) = said {
        return Some(mark(problem, draft.submits.0));
    }
    problem_of(specs, draft)
        .filter(|problem| problem.problem == ProblemKind::Invalid)
        .map(|problem| mark(problem, draft.submits.0))
}

/// The part of the server form a field sits in.
fn part_of(kind: FieldKind) -> FormPart {
    match kind {
        FieldKind::Protocol
        | FieldKind::Server
        | FieldKind::Security
        | FieldKind::Port
        | FieldKind::SessionUrl => FormPart::Incoming,
        FieldKind::OutgoingServer | FieldKind::OutgoingSecurity | FieldKind::OutgoingPort => {
            FormPart::Outgoing
        }
        FieldKind::Address
        | FieldKind::Username
        | FieldKind::Password
        | FieldKind::ApiKey
        | FieldKind::Token => FormPart::SignIn,
    }
}

/// The options of a choice field, each of porter's slugs in mailo's words.
fn choices_of(kind: FieldKind) -> Vec<Choice> {
    kind.choices()
        .iter()
        .map(|slug| Choice::new(*slug, choice_label(slug)))
        .collect()
}

/// What a choice's slug is called.
pub(super) fn choice_label(slug: &str) -> String {
    match slug {
        "imap" => "IMAP",
        "pop3" => "POP3",
        "jmap" => "JMAP",
        "tls" => "SSL/TLS",
        "starttls" => "STARTTLS",
        other => other,
    }
    .to_owned()
}

/// The port a server usually listens on, for the protocol and security the draft holds: the
/// number a port's empty entry shows.
fn usual_port(port: FieldKind, draft: &Draft) -> u16 {
    let plain = |kind| {
        draft
            .fields
            .iter()
            .find(|field| field.kind == kind)
            .and_then(|field| match &field.value {
                FieldValue::Plain(text) => Some(text.as_str()),
                FieldValue::Secret(_) => None,
            })
    };
    let outgoing = port == FieldKind::OutgoingPort;
    let secure = plain(match outgoing {
        true => FieldKind::OutgoingSecurity,
        false => FieldKind::Security,
    }) != Some("starttls");
    match (outgoing, plain(FieldKind::Protocol), secure) {
        (true, _, true) => 465,
        (true, _, false) => 587,
        (false, Some("pop3"), true) => 995,
        (false, Some("pop3"), false) => 110,
        (false, _, true) => 993,
        (false, _, false) => 143,
    }
}

fn copy_state(draft: &Draft) -> CopyState {
    match draft.copied {
        CopyMark::Idle => CopyState::Idle,
        CopyMark::Copied => CopyState::Copied,
    }
}

// ---------------------------------------------------------------------------------------------
// Events: what the parts report, as an action.

/// A row of the provider list.
pub(super) fn list_key(pick: &ProviderPick) -> Option<ListKey> {
    match pick {
        ProviderPick::Provider(key) => ProviderId::parse(&key.0).ok().map(ListKey::Provider),
        ProviderPick::Other => Some(ListKey::Other),
    }
}

/// A form field changed.
pub(super) fn typed(role: FieldRole, text: FieldText) -> Action {
    let value = match text {
        FieldText::Plain(text) => FieldValue::Plain(text),
        FieldText::Secret(text) => FieldValue::Secret(SecretText::new(text.reveal())),
    };
    Action::Type(kind_of(role), value)
}

// ---------------------------------------------------------------------------------------------
// Words and marks.

/// The role a porter field kind plays in the form.
pub(super) fn role_of(kind: FieldKind) -> FieldRole {
    match kind {
        FieldKind::Address => FieldRole::Address,
        FieldKind::Server => FieldRole::Server,
        FieldKind::Username => FieldRole::Username,
        FieldKind::Password => FieldRole::Password,
        FieldKind::ApiKey => FieldRole::ApiKey,
        FieldKind::Token => FieldRole::Token,
        FieldKind::Protocol => FieldRole::Protocol,
        FieldKind::Port => FieldRole::Port,
        FieldKind::Security => FieldRole::Security,
        FieldKind::OutgoingServer => FieldRole::OutgoingServer,
        FieldKind::OutgoingPort => FieldRole::OutgoingPort,
        FieldKind::OutgoingSecurity => FieldRole::OutgoingSecurity,
        FieldKind::SessionUrl => FieldRole::SessionUrl,
    }
}

/// The field kind a form role stands for (the way back).
pub(super) fn kind_of(role: FieldRole) -> FieldKind {
    match role {
        FieldRole::Address => FieldKind::Address,
        FieldRole::Server => FieldKind::Server,
        FieldRole::Username => FieldKind::Username,
        FieldRole::Password => FieldKind::Password,
        FieldRole::ApiKey => FieldKind::ApiKey,
        FieldRole::Token => FieldKind::Token,
        FieldRole::Protocol => FieldKind::Protocol,
        FieldRole::Port => FieldKind::Port,
        FieldRole::Security => FieldKind::Security,
        FieldRole::OutgoingServer => FieldKind::OutgoingServer,
        FieldRole::OutgoingPort => FieldKind::OutgoingPort,
        FieldRole::OutgoingSecurity => FieldKind::OutgoingSecurity,
        FieldRole::SessionUrl => FieldKind::SessionUrl,
    }
}

/// What a provider is called, for the titles of the steps after the list (the list carries its own
/// labels): the provider files' labels, and the id itself for any other.
pub(super) fn provider_label(id: &ProviderId) -> String {
    match id.as_str() {
        "fastmail" => "Fastmail",
        "generic-dav" => "your calendar and contacts server",
        "generic-imap" | "generic-jmap" => "your email server",
        "gmx" => "GMX",
        "google" | "google-mail" => "Google",
        "icloud" => "iCloud",
        "microsoft" => "Microsoft",
        "yahoo" => "Yahoo Mail",
        other => other,
    }
    .to_owned()
}

/// The mark a provider wears, from a provider file's `mark` word or its id. Providers without a
/// mark of their own wear the neutral one.
pub(super) fn mark_of(mark: &str) -> MarkProvider {
    match mark {
        "google" | "google-mail" | "gemini" | "google-ai" => MarkProvider::Google,
        "microsoft" => MarkProvider::Microsoft,
        "fastmail" => MarkProvider::Fastmail,
        "icloud" => MarkProvider::ICloud,
        "yahoo" => MarkProvider::Yahoo,
        "local" => MarkProvider::Local,
        _ => MarkProvider::Imap,
    }
}

/// What a kind of service is called on the review.
pub(super) fn service_name(kind: CapabilityKind) -> &'static str {
    match kind {
        CapabilityKind::Identity => "Account details",
        CapabilityKind::Mail => "Mail",
        CapabilityKind::Calendar => "Calendar",
        CapabilityKind::Contacts => "Contacts",
        CapabilityKind::Tasks => "Tasks",
        CapabilityKind::Notes => "Notes",
        CapabilityKind::Storage => "Files",
        CapabilityKind::Photos => "Photos",
        CapabilityKind::Llm => "Language model",
        CapabilityKind::Embeddings => "Search by meaning",
        CapabilityKind::Speech => "Speech",
        CapabilityKind::ImageGen => "Image generation",
        CapabilityKind::Rerank => "Result ranking",
        CapabilityKind::ComputerUse => "Operating windows",
        CapabilityKind::KeyValue => "Small synced items",
        CapabilityKind::Push => "Notifications",
    }
}

/// The review's secondary line for a service the provider offers but limits.
pub(super) fn limit_of(reason: LimitReason) -> Limitation {
    match reason {
        LimitReason::AppendOnly => Limitation::AppendOnly,
        LimitReason::PickerOnly => Limitation::PickerOnly,
        LimitReason::AppFolderOnly => Limitation::AppFolderOnly,
        LimitReason::ProviderOffersNone => Limitation::ProviderOffersNone,
    }
}

/// Why a service is absent.
pub(super) fn absence_of(reason: AbsentReason) -> Limitation {
    match reason {
        AbsentReason::ProviderOffersNone => Limitation::ProviderOffersNone,
        AbsentReason::TenantConsent => Limitation::AdminConsent,
        AbsentReason::UnverifiedBuild => Limitation::UnverifiedBuild,
        AbsentReason::TurnedOff => Limitation::TurnedOff,
        AbsentReason::NotOnServer => Limitation::NotOnServer,
    }
}

/// Why a sign-in ended without an account.
pub(super) fn fault_of(fault: SignInFault) -> ShellFault {
    match fault {
        SignInFault::Refused => ShellFault::Refused,
        SignInFault::Unreachable => ShellFault::Unreachable,
        SignInFault::Unreadable => ShellFault::Unreadable,
        SignInFault::NeedsClientId => ShellFault::NeedsClientId,
        SignInFault::TimedOut => ShellFault::TimedOut,
        SignInFault::Cancelled => ShellFault::Cancelled,
        SignInFault::Forbidden => ShellFault::Forbidden,
        SignInFault::StoreFailed => ShellFault::StoreFailed,
    }
}

/// The key a review row goes by: the kind's slug on the wire.
pub(super) fn service_key(kind: CapabilityKind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(slug)) => slug,
        _ => String::new(),
    }
}
