//! The Add account sheet: an address, what was found for it — or a JMAP server typed in by hand
//! — a password, a token or a browser sign-in, and the one press that uses them.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_domain::HttpAuth;
use mail_store::SqliteStore;

use super::super::field::{Field, FieldKind};
use super::super::hover::copy;
use super::super::press::{SheetClose, available, on_primary};
use super::flow::{self, Client, Hand, Offer, Opened, SignIn, SigningIn, Stage};
use crate::password::Password;
use crate::space::Spaces;
use crate::view::Shell;
use ds::{Button, ButtonVariant, Icon, InputVariant, SegmentedControl, TextInput};

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
        div {
            class: "files-wrap",
            onclick: move |_| super::close(shell),
            div {
                class: "files acct-sheet",
                role: "dialog",
                aria_label: "Add account",
                onclick: move |event| event.stop_propagation(),
                div { class: "files-head",
                    h3 { "Add account" }
                    SheetClose { on_close: move |()| super::close(shell) }
                }
                div { class: "files-main",
                    span { class: "files-k", "Address" }
                    Field {
                        kind: FieldKind::Boxed,
                        value: typed.clone(),
                        placeholder: "you@example.com".to_owned(),
                        extra: Some("acct-in".to_owned()),
                        on_input: on_address,
                        on_focus: |_| {},
                        on_blur: |_| {},
                    }
                    Below { shown: shown.clone(), typed: typed.clone(), signing: signing(), stage, secret, on_secret: move |value: String| {
                        secret.set(Password::new(value));
                    } }
                }
                div { class: "files-foot",
                    ds::Button {
                        variant: ds::ButtonVariant::Mini,
                        label: dismiss.to_string(),
                        aria_label: dismiss.to_string(),
                        onclick: on_primary(move || super::close(shell)),
                    }
                    ds::Button {
                        variant: ds::ButtonVariant::Primary,
                        extra_class: ds::ExtraClass::parse("go").ok(),
                        label: primary.to_string(),
                        icon,
                        aria_label: primary.to_string(),
                        availability: available(enabled),
                        onclick: on_primary(move || press(())),
                    }
                }
            }
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
            p { class: "capnote", "{flow::before_looking(&typed, chrono::Utc::now())}" }
            Alternate { stage, secret }
        },
        Stage::Looking => rsx! {
            p { class: "files-look acct-busy", role: "status",
                "Looking up the servers for {flow::domain_of(&typed).unwrap_or_default()}…"
            }
        },
        Stage::Missed(why) => rsx! {
            p { class: "capnote files-bad acct-said", role: "alert", "{why}" }
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
            p { class: "capnote files-bad acct-said", role: "alert", "{why}" }
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
                        "Saving the account and putting the {} in the system keyring…",
                        if offer.token() { "token" } else { "password" }
                    ),
                    SignIn::OAuth { issuer, .. } => {
                        format!("Starting the sign-in with {}…", flow::provider(issuer))
                    }
                };
                rsx! {
                    Found { offer }
                    p { class: "files-look acct-busy", role: "status", "{waiting}" }
                }
            }
        },
        Stage::Added { said, .. } => rsx! {
            div { class: "acct-done", role: "status",
                for line in said {
                    p { "{line}" }
                }
            }
        },
    }
}

/// What was found: incoming, outgoing, sign-in, and where it came from.
#[component]
fn Found(offer: Offer) -> Element {
    rsx! {
        div { class: "acct-found",
            for (what, how) in offer.rows.iter() {
                span { class: "acct-k", "{what}" }
                span { class: "acct-v", "{how}" }
            }
            span { class: "acct-k", "found" }
            span { class: "acct-v acct-source", "{offer.source}" }
        }
        p { class: "capnote", "Nothing has been sent to these servers yet." }
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
            span { class: "files-k", "Connect with" }
            SegmentedControl::<bool> {
                label: "Connect with",
                options,
                value: jmap.is_some(),
                onchange: on_way,
            }
            p { class: "capnote", "{said}" }
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
    let options = vec![
        (HttpAuth::Basic, "Password".to_owned()),
        (HttpAuth::Bearer, "API token".to_owned()),
    ];
    rsx! {
        span { class: "files-k", "Signs in with" }
        SegmentedControl::<HttpAuth> {
            label: "Signs in with",
            options,
            value: auth,
            onchange: move |want: HttpAuth| {
                let next = flow::pick_auth(&stage.peek(), want);
                if let Some(next) = next {
                    stage.set(next);
                }
            },
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
        span { class: "files-k", "JMAP session URL" }
        TextInput {
            variant: InputVariant::Boxed,
            label: "JMAP session URL",
            value: hand.session.clone(),
            placeholder: "https://jmap.example.com/.well-known/jmap",
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
        p { class: "capnote",
            "The address your provider gives for JMAP. Nothing is looked up, and nothing is sent to it until you use these settings."
        }
        if let Some(why) = why {
            p { class: "capnote files-bad acct-said", role: "alert", "{why}" }
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
                variant: ButtonVariant::Quiet,
                label: label.to_owned(),
                id: Some(id.to_owned()),
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
            p { class: "capnote",
                "No password: you sign in with {flow::provider(issuer)} in your browser, and mailo keeps only the sign-in it is given, in the system keyring."
            }
        },
        SignIn::OAuth {
            issuer,
            client: Client::Missing,
        } => rsx! {
            p { class: "capnote files-bad acct-said", "{flow::missing_client(issuer)}" }
        },
    }
}

/// A password or token field. What is typed goes to `on_secret` and is never drawn back.
#[component]
fn Secret(address: String, token: bool, on_secret: EventHandler<String>) -> Element {
    let what = if token { "API token" } else { "Password" };
    rsx! {
        span { class: "files-k", "{what}" }
        Field {
            kind: FieldKind::Secret,
            value: String::new(),
            placeholder: format!("{what} for {address}"),
            extra: Some("acct-in".to_owned()),
            on_input: move |value: String| on_secret.call(value),
            on_focus: |_| {},
            on_blur: |_| {},
        }
        p { class: "capnote",
            "It goes to the system keyring, and to these servers when mailo signs in. It is never saved anywhere else."
        }
    }
}

/// A sign-in waiting in the browser: the address it waits on, to select or copy, and whether a
/// browser was opened there.
#[component]
fn Browser(signing: SigningIn) -> Element {
    let SigningIn { url, opened } = signing;
    let copy_label = "Copy";
    let how = match opened {
        Opened::Browser => "If no browser opened, open this address in one:".to_owned(),
        Opened::Not(why) => {
            format!("mailo could not open a browser ({why}). Open this address in one:")
        }
    };
    rsx! {
        p { class: "files-look acct-busy", role: "status", "Finish signing in in your browser" }
        p { class: "capnote", "{how}" }
        div { class: "acct-url",
            span { class: "acct-link", "{url}" }
            ds::Button {
                variant: ds::ButtonVariant::Mini,
                label: copy_label.to_string(),
                aria_label: copy_label.to_string(),
                onclick: on_primary(move || copy(&url)),
            }
        }
    }
}
