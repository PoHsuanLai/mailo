//! The Add account sheet: an address, what was found for it — or a JMAP server typed in by hand
//! — a password, a token or a browser sign-in, and the one press that uses them.

use std::sync::Arc;

use super::super::common::in_card;
use dioxus::prelude::*;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::Answers;
use ds::components::controls::button_model::Bezel;
use ds::components::controls::progress::model::Progress;
use ds::components::controls::progress::view::ProgressIndicator;
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::{FieldRow, RowLayout};
use ds::components::overlays::sheet_attach::Attach;
use ds::motion::detail::operation::{Operation, PendingToken};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::HttpAuth;
use mail_store::SqliteStore;

use super::super::hover::copy;
use super::super::press::{available, on_primary};
use super::flow::{self, Client, Hand, Offer, Opened, SignIn, SigningIn, Stage};
use crate::password::Password;
use crate::space::Spaces;
use crate::view::Shell;

/// Look up what is typed, off the thread that draws. Call it from an event handler (F140).
fn look_up(shell: Signal<Shell>, mut stage: Signal<Stage>) {
    if stage.peek().busy() {
        return;
    }
    let typed = shell.peek().adding.clone().unwrap_or_default();
    let seams = super::seams();
    stage.set(Stage::Looking);
    spawn(async move {
        let done =
            tokio::task::spawn_blocking(move || flow::look(&typed, &seams, chrono::Utc::now()))
                .await;
        stage.set(done.unwrap_or_else(|error| {
            Stage::Missed(format!("The lookup stopped before it finished: {error}"))
        }));
    });
}

/// Use `offer`, found or typed in: the password or token is taken out of the sheet, not copied,
/// and handed to the add, which drops it when it is done. Call it from an event handler (F140).
///
/// A browser sign-in's address comes back from the blocking add over a channel, is opened in the
/// system browser there, and lands in `signing` for the sheet to show while it waits.
fn use_offer(
    offer: Offer,
    shell: Signal<Shell>,
    mut stage: Signal<Stage>,
    mut secret: Signal<Password>,
    mut revision: Signal<u64>,
    mut spaces: Signal<Spaces>,
    mut signing: Signal<Option<SigningIn>>,
) {
    let password = std::mem::take(&mut *secret.write());
    let seams = super::seams();
    let store = consume_context::<Arc<SqliteStore>>();
    let kept = offer.clone();
    stage.set(Stage::Adding(offer.clone()));
    signing.set(None);
    let (sender, mut received) = tokio::sync::mpsc::unbounded_channel();
    // Ends when the add does, which drops the sender.
    spawn(async move {
        while let Some(now) = received.recv().await {
            signing.set(Some(now));
        }
    });
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            let on_url = |url: &str| {
                // The sheet is gone if nobody is receiving, and then there is nobody to tell.
                let _ = sender.send(flow::signing_in(url, seams.browse.as_ref()));
            };
            flow::confirm(&store, offer, password, seams.add.as_ref(), &on_url)
        })
        .await;
        let next = done.unwrap_or_else(|error| {
            Stage::Refused(kept, format!("It stopped before it finished: {error}"))
        });
        if let Stage::Added { account, .. } = &next {
            revision += 1;
            if let Some(account) = *account {
                into_scope(shell, &mut spaces, account);
            }
        }
        stage.set(next);
    });
}

/// A new account joins the current Space when the Space is limited to some accounts.
fn into_scope(
    mut shell: Signal<Shell>,
    spaces: &mut Signal<Spaces>,
    account: mail_domain::AccountId,
) {
    let widened = {
        let mut all = spaces.write();
        let current = all.current;
        all.spaces
            .get_mut(current)
            .is_some_and(|space| flow::widen(space, account))
    };
    if widened {
        super::super::frame::keep(&spaces.read());
        shell.write().scope = super::super::frame::scope_ids(&spaces.read().current_space());
    }
}

/// The sheet. Mounted while `shell.adding` is `Some`, which holds the address.
#[component]
pub(in crate::ui) fn AddAccountSheet(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    spaces: Signal<Spaces>,
) -> Element {
    let typed = shell.read().adding.clone().unwrap_or_default();
    let mut stage = use_signal(|| Stage::Blank);
    // Held here and only here, so it goes when the sheet does.
    let mut secret = use_signal(Password::default);
    let signing = use_signal(|| None::<SigningIn>);
    let has_password = !secret.read().is_empty();
    let shown = stage.read().clone();
    let (primary, enabled) = match &shown {
        Stage::Blank | Stage::Missed(_) => {
            ("Look up".to_owned(), flow::domain_of(&typed).is_some())
        }
        Stage::Looking => ("Looking up…".to_owned(), false),
        Stage::Found(offer) | Stage::Refused(offer, _) => match offer.sign_in {
            SignIn::Password => ("Use these settings".to_owned(), has_password),
            SignIn::OAuth { issuer, client } => (
                format!("Sign in with {}", flow::provider(issuer)),
                client == Client::Ready,
            ),
        },
        Stage::ByHand(hand) => (
            "Use these settings".to_owned(),
            has_password && flow::by_hand(&typed, hand).is_ok(),
        ),
        Stage::Adding(offer) => match offer.sign_in {
            SignIn::Password => ("Adding…".to_owned(), false),
            SignIn::OAuth { .. } => ("Waiting for the browser…".to_owned(), false),
        },
        Stage::Added { .. } => ("Done".to_owned(), true),
    };
    let dismiss = if matches!(shown, Stage::Added { .. }) {
        "Close"
    } else {
        "Cancel"
    };
    let press = move |_| {
        // Read and let go before acting: each action writes the stage.
        let now = stage.peek().clone();
        match now {
            Stage::Blank | Stage::Missed(_) => look_up(shell, stage),
            Stage::Found(offer) | Stage::Refused(offer, _) => {
                use_offer(offer, shell, stage, secret, revision, spaces, signing)
            }
            Stage::ByHand(hand) => {
                let typed = shell.peek().adding.clone().unwrap_or_default();
                if let Ok(offer) = flow::by_hand(&typed, &hand) {
                    use_offer(offer, shell, stage, secret, revision, spaces, signing)
                }
            }
            Stage::Added { .. } => super::close(shell),
            Stage::Looking | Stage::Adding(_) => {}
        }
    };
    let on_address = move |value: String| {
        shell.write().adding = Some(value);
        // What was found was for the address as it was, and so was what was typed for it; a
        // server typed in by hand stays.
        if matches!(
            *stage.peek(),
            Stage::Found(_) | Stage::Missed(_) | Stage::Refused(..)
        ) {
            stage.set(Stage::Blank);
            secret.set(Password::default());
        }
    };
    let icon = match &shown {
        Stage::Found(Offer {
            sign_in: SignIn::OAuth { .. },
            ..
        }) => Icon::Key,
        Stage::Added { .. } => Icon::Check,
        Stage::Found(_) | Stage::Refused(..) | Stage::Adding(_) | Stage::ByHand(_) => Icon::Plus,
        _ => Icon::Search,
    };
    rsx! {
        Sheet {
            label: "Add account",
            attach: Attach::Window,
            common: in_card(),
            onclose: move |()| super::close(shell),
            div { class: "acct-sheet",
                Label { text: "Add Account", style: LabelStyle::Title }
                FieldRow {
                    label: "Address",
                    layout: RowLayout::Form,
                    TextField {
                        label: "Address",
                        value: typed.clone(),
                        placeholder: "you@example.com".to_owned(),
                        focus: FieldFocus::OnMount,
                        oninput: on_address,
                    }
                }
                Below { shown: shown.clone(), typed: typed.clone(), signing: signing(), stage, secret, on_secret: move |value: String| {
                    secret.set(Password::new(value));
                } }
                div { class: "acct-foot",
                    Button {
                        label: dismiss.to_string(),
                        common: Common { aria_label: Some(dismiss.to_string()), ..Common::default() },
                        answers: Answers::Escape,
                        onclick: on_primary(move || super::close(shell)),
                    }
                    Button {
                        label: primary.to_string(),
                        common: Common { aria_label: Some(primary.to_string()), ..Common::default() },
                        icon,
                        answers: Answers::Return,
                        availability: available(enabled),
                        onclick: on_primary(move || press(())),
                    }
                }
            }
        }
    }
}

/// A line of small explanation or a refusal under a row.
#[component]
fn Note(text: String, tone: NoteTone) -> Element {
    rsx! {
        Label {
            text,
            role: LabelRole::Secondary,
            style: LabelStyle::Footnote,
            common: super::super::sidebar::tagged("note", tone.slug()),
        }
    }
}

/// What a note says of itself: help, or that something went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteTone {
    Help,
    Refusal,
}

impl NoteTone {
    fn slug(self) -> &'static str {
        match self {
            NoteTone::Help => "help",
            NoteTone::Refusal => "refusal",
        }
    }
}

/// Work in progress: quire's spinner beside what is being done, said as a status.
#[component]
fn Busy(text: String) -> Element {
    // One operation for as long as the line is shown.
    let token = use_hook(PendingToken::start);
    rsx! {
        div { class: "acct-busy", role: "status",
            ProgressIndicator {
                style: ds::components::controls::progress::model::ProgressStyle::Spinner,
                progress: Progress::Unknown(Operation::Running(token)),
                size: ControlSize::Small,
            }
            Label { text, role: LabelRole::Secondary }
        }
    }
}

/// Everything under the address, for where the sheet stands.
#[component]
fn Below(
    shown: Stage,
    typed: String,
    signing: Option<SigningIn>,
    stage: Signal<Stage>,
    secret: Signal<Password>,
    on_secret: EventHandler<String>,
) -> Element {
    match shown {
        Stage::Blank => rsx! {
            Note { text: flow::before_looking(&typed, chrono::Utc::now()), tone: NoteTone::Help }
            Alternate { stage, secret }
        },
        Stage::Looking => rsx! {
            Busy { text: format!("Looking up the servers for {}\u{2026}", flow::domain_of(&typed).unwrap_or_default()) }
        },
        Stage::Missed(why) => rsx! {
            Note { text: why, tone: NoteTone::Refusal }
            Alternate { stage, secret }
        },
        Stage::Found(offer) => rsx! {
            Found { offer: offer.clone() }
            Ways { offer: offer.clone(), stage, secret }
            Credential { offer, on_secret }
            Alternate { stage, secret }
        },
        Stage::Refused(offer, why) => rsx! {
            Found { offer: offer.clone() }
            Ways { offer: offer.clone(), stage, secret }
            Credential { offer, on_secret }
            Note { text: why, tone: NoteTone::Refusal }
            Alternate { stage, secret }
        },
        Stage::ByHand(hand) => rsx! {
            ByHand { hand, typed, stage, on_secret }
            Alternate { stage, secret }
        },
        Stage::Adding(offer) => match (offer.sign_in, signing) {
            (SignIn::OAuth { .. }, Some(signing)) => rsx! {
                Found { offer }
                Browser { signing }
            },
            (sign_in, _) => {
                let waiting = match sign_in {
                    SignIn::Password => format!(
                        "Saving the account and putting the {} in the system keyring\u{2026}",
                        if offer.token() { "token" } else { "password" }
                    ),
                    SignIn::OAuth { issuer, .. } => {
                        format!(
                            "Starting the sign-in with {}\u{2026}",
                            flow::provider(issuer)
                        )
                    }
                };
                rsx! {
                    Found { offer }
                    Busy { text: waiting }
                }
            }
        },
        Stage::Added { said, .. } => rsx! {
            div { class: "acct-done", role: "status",
                for (index, line) in said.into_iter().enumerate() {
                    Label {
                        text: line,
                        style: if index == 0 { LabelStyle::Headline } else { LabelStyle::Body },
                    }
                }
            }
        },
    }
}

/// What was found: incoming, outgoing, sign-in, and where it came from, one form row each.
#[component]
fn Found(offer: Offer) -> Element {
    rsx! {
        for (what, how) in offer.rows.iter() {
            FieldRow { label: what.clone(), layout: RowLayout::Form,
                Label { text: how.clone(), common: super::super::sidebar::tagged("found", what.clone()) }
            }
        }
        FieldRow { label: "Found", layout: RowLayout::Form,
            Label { text: offer.source.clone(), role: LabelRole::Secondary }
        }
        Note { text: "Nothing has been sent to these servers yet.".to_owned(), tone: NoteTone::Help }
    }
}

/// For a domain that offered two ways, which one to use; for a JMAP offer, whether the secret
/// is a password or an API token. Nothing for any other offer.
#[component]
fn Ways(offer: Offer, stage: Signal<Stage>, secret: Signal<Password>) -> Element {
    let choice = flow::both(&offer).map(|said| (said, flow::ways(&offer)));
    let jmap = offer.jmap_auth();
    let sign_in = offer.sign_in;
    let on_way = move |want: bool| {
        let next = flow::pick_way(&stage.peek(), want);
        if let Some(next) = next {
            // A secret field that stays keeps what was typed into it; one that goes, or
            // comes new and empty, takes it along.
            let kept = matches!(&next, Stage::Found(o) if o.sign_in == sign_in);
            if !kept {
                secret.set(Password::default());
            }
            stage.set(next);
        }
    };
    rsx! {
        if let Some((said, options)) = choice {
            FieldRow { label: "Connect with", help: said, layout: RowLayout::Form,
                SegmentedControl::<bool> {
                    label: "Connect with",
                    choices: options
                        .into_iter()
                        .map(|(value, name)| Choice::new(value, name))
                        .collect::<Vec<_>>(),
                    tracking: Tracking::SelectOne(jmap.is_some()),
                    onchange: on_way,
                }
            }
        }
        if let Some(auth) = jmap {
            SignsInWith { auth, stage }
        }
    }
}

/// A JMAP secret: a password, sent with HTTP Basic, or an API token the provider issued, sent
/// as a bearer token.
#[component]
fn SignsInWith(auth: HttpAuth, stage: Signal<Stage>) -> Element {
    let choices = vec![
        Choice::new(HttpAuth::Basic, "Password"),
        Choice::new(HttpAuth::Bearer, "API token"),
    ];
    rsx! {
        FieldRow { label: "Signs in with", layout: RowLayout::Form,
            SegmentedControl::<HttpAuth> {
                label: "Signs in with",
                choices,
                tracking: Tracking::SelectOne(auth),
                onchange: move |want: HttpAuth| {
                    let next = flow::pick_auth(&stage.peek(), want);
                    if let Some(next) = next {
                        stage.set(next);
                    }
                },
            }
        }
    }
}

/// A JMAP server typed in: its session URL, how it signs in, and the secret.
#[component]
fn ByHand(
    hand: Hand,
    typed: String,
    stage: Signal<Stage>,
    on_secret: EventHandler<String>,
) -> Element {
    let address = typed.trim().to_lowercase();
    // Said once there is something to say it about.
    let why = flow::by_hand(&typed, &hand)
        .err()
        .filter(|_| !hand.session.trim().is_empty());
    rsx! {
        FieldRow {
            label: "JMAP session URL",
            help: "The address your provider gives for JMAP. Nothing is looked up, and nothing is sent to it until you use these settings.",
            layout: RowLayout::Form,
            TextField {
                label: "Session URL",
                value: hand.session.clone(),
                placeholder: "https://jmap.example.com/.well-known/jmap".to_owned(),
                oninput: move |value: String| {
                    let next = match &*stage.peek() {
                        Stage::ByHand(now) => Stage::ByHand(Hand {
                            session: value,
                            ..now.clone()
                        }),
                        _ => return,
                    };
                    stage.set(next);
                },
            }
        }
        if let Some(why) = why {
            Note { text: why, tone: NoteTone::Refusal }
        }
        SignsInWith { auth: hand.auth, stage }
        Secret { address, token: hand.auth == HttpAuth::Bearer, on_secret }
    }
}

/// Between looking up and typing a JMAP server in. What was typed for the one goes with it.
#[component]
fn Alternate(stage: Signal<Stage>, mut secret: Signal<Password>) -> Element {
    let (id, label) = if matches!(*stage.read(), Stage::ByHand(_)) {
        ("acct-look-instead", "Look the address up instead")
    } else {
        ("acct-by-hand", "Enter a JMAP server by hand")
    };
    rsx! {
        div { class: "acct-alt",
            Button {
                bezel: Bezel::Inline,
                label: label.to_owned(),
                common: Common { id: Some(id.to_owned()), ..Common::default() },
                onclick: move |_| {
                    let next = flow::alternate(&stage.peek());
                    secret.set(Password::default());
                    stage.set(next);
                },
            }
        }
    }
}

/// The password or token field, or what a browser sign-in will do.
#[component]
fn Credential(offer: Offer, on_secret: EventHandler<String>) -> Element {
    match offer.sign_in {
        SignIn::Password => rsx! {
            Secret { address: offer.address.clone(), token: offer.token(), on_secret }
        },
        SignIn::OAuth {
            issuer,
            client: Client::Ready,
        } => rsx! {
            Note {
                text: format!("No password: you sign in with {} in your browser, and mailo keeps only the sign-in it is given, in the system keyring.", flow::provider(issuer)),
                tone: NoteTone::Help,
            }
        },
        SignIn::OAuth {
            issuer,
            client: Client::Missing,
        } => rsx! {
            Note { text: flow::missing_client(issuer), tone: NoteTone::Refusal }
        },
    }
}

/// A password or token field. What is typed goes to `on_secret` and is never drawn back.
#[component]
fn Secret(address: String, token: bool, on_secret: EventHandler<String>) -> Element {
    let what = if token { "API token" } else { "Password" };
    rsx! {
        FieldRow {
            label: what,
            help: "It goes to the system keyring, and to these servers when mailo signs in. It is never saved anywhere else.",
            layout: RowLayout::Form,
            TextField {
                label: what,
                kind: FieldKind::Secure,
                value: String::new(),
                placeholder: format!("{what} for {address}"),
                oninput: move |value: String| on_secret.call(value),
            }
        }
    }
}

/// A sign-in waiting in the browser: the address it waits on, to select or copy, and whether a
/// browser was opened there.
#[component]
fn Browser(signing: SigningIn) -> Element {
    let SigningIn { url, opened } = signing;
    let how = match opened {
        Opened::Browser => "If no browser opened, open this address in one:".to_owned(),
        Opened::Not(why) => {
            format!("mailo could not open a browser ({why}). Open this address in one:")
        }
    };
    rsx! {
        Busy { text: "Finish signing in in your browser".to_owned() }
        Note { text: how, tone: NoteTone::Help }
        div { class: "acct-url",
            Label { text: url.clone(), style: LabelStyle::Footnote, common: Common { extra_class: ds::root::pass_through::ExtraClass::parse("acct-link").ok(), ..Common::default() } }
            Button {
                label: "Copy",
                common: Common { aria_label: Some("Copy".to_owned()), ..Common::default() },
                onclick: on_primary(move || copy(&url)),
            }
        }
    }
}
