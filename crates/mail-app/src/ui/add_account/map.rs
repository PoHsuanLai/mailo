//! The add-account window's one mapping: porter's `SheetView` to the props of quire's
//! `ds-shell::accounts` parts, and what the parts report back to the `SheetInput` the service
//! is told. The same mapping sill's accounts sheet makes (`sill-overlays/src/accounts_sheet`), in
//! this window: the sheet is controlled, so the window owns every value a step shows (what is
//! typed, the cursor, the switches) in a [`Draft`], which with its transitions
//! (`mail_core::account::draft`: what a view, a draft and an action make of each other) is
//! `mail-core`'s. What is here is the other half, the mapping to and from the parts. Nothing
//! here draws, reads a secret store or talks to a service.
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
pub(super) use mail_core::account::draft::{Action, Out, Sheet, acted, shown};
use mail_core::account::draft::{CopyMark, Draft, FieldDraft, ListKey, is_manual, problem_of};
use porter_core::sheet::{
    FieldKind, FieldProblem, FieldSpec, FieldValue, Presence, ProblemKind, RowKind, ServiceState,
    SheetView, SignInFault,
};
use porter_core::{AbsentReason, CapabilityKind, LimitReason, ProviderId, SecretText};

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
        mark: MarkProvider,
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
        mark: MarkProvider,
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
                // gap(quire): porter's `row.mark_face` has nowhere to go until ProviderEntry
                // takes a face (`.faced`, quire v0.2.31); the mark id picks a known one.
                .map(|row| {
                    ProviderEntry::new(
                        ProviderKey(row.id.as_str().to_owned()),
                        row.label.clone(),
                        mark_of(&row.mark),
                    )
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
        SheetView::BrowserWait { provider, url, .. } => Step::Browser {
            mark: mark_of(provider.as_str()),
            provider: provider_label(provider),
            url: url.as_str().to_owned(),
            copied: copy_state(draft),
        },
        SheetView::ShowCode {
            provider,
            user_code,
            url,
            ..
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
        SheetView::Working { provider, .. } => Step::Working {
            mark: mark_of(provider.as_str()),
            provider: provider_label(provider),
        },
        SheetView::Failed {
            provider, fault, ..
        } => Step::Failed {
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
        | FieldKind::AppPassword
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
        // The plain name first and the protocol second, as quire's own forms word them.
        "imap" => "Most servers (IMAP)",
        "pop3" => "Older servers (POP)",
        "jmap" => "Newer servers (JMAP)",
        "tls" => "Secure from the start (SSL/TLS)",
        "starttls" => "Secure after connecting (STARTTLS)",
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
        FieldKind::AppPassword => FieldRole::AppPassword,
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
        FieldRole::AppPassword => FieldKind::AppPassword,
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

/// What a kind of service is called on the review: porter's own name for it, so mailo's review
/// and porter's say the same.
pub(super) fn service_name(kind: CapabilityKind) -> &'static str {
    kind.display_name()
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
        // An agent's own login, never a mail account's; said as quire's sheet says them.
        SignInFault::NoLauncher => ShellFault::NoLauncher,
        SignInFault::NotInstalled => ShellFault::NotInstalled,
        // The launcher did not answer in time: the person waited and nothing came.
        SignInFault::Expired => ShellFault::TimedOut,
        SignInFault::AlreadyAdded => ShellFault::AlreadyAdded,
        // A program on this computer that holds its own sign-in (Tailscale): never a mail
        // account's either, said as quire's sheet says them.
        SignInFault::NotRunning => ShellFault::NotRunning,
        SignInFault::SignedOut => ShellFault::SignedOut,
        SignInFault::NotAllowed => ShellFault::NotAllowed,
        // A fault newer than this build: the sheet says it could not read the answer.
        _ => ShellFault::Unreadable,
    }
}

/// The key a review row goes by: the kind's slug on the wire.
pub(super) fn service_key(kind: CapabilityKind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(slug)) => slug,
        _ => String::new(),
    }
}
