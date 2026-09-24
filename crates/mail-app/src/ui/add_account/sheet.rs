//! The Add account sheet: an address, what was found for it, a password or a browser sign-in,
//! and the one press that uses them.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::SqliteStore;

use super::super::field::{Field, FieldKind};
use super::super::hover::copy;
use super::flow::{self, Client, Offer, Opened, SignIn, SigningIn, Stage};
use crate::password::Password;
use crate::space::Spaces;
use crate::view::Shell;
use ds::{Glyph, Icon};

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

/// Use what was found: the password is taken out of the sheet, not copied, and handed to the
/// add, which drops it when it is done. Call it from an event handler (F140).
///
/// A browser sign-in's address comes back from the blocking add over a channel, is opened in the
/// system browser there, and lands in `signing` for the sheet to show while it waits.
fn use_offer(
    shell: Signal<Shell>,
    mut stage: Signal<Stage>,
    mut secret: Signal<Password>,
    mut revision: Signal<u64>,
    mut spaces: Signal<Spaces>,
    mut signing: Signal<Option<SigningIn>>,
) {
    let offer = match &*stage.peek() {
        Stage::Found(offer) | Stage::Refused(offer, _) => offer.clone(),
        _ => return,
    };
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
            Stage::Found(_) | Stage::Refused(..) => {
                use_offer(shell, stage, secret, revision, spaces, signing)
            }
            Stage::Added { .. } => super::close(shell),
            Stage::Looking | Stage::Adding(_) => {}
        }
    };
    let on_address = move |value: String| {
        shell.write().adding = Some(value);
        // What was found was for the address as it was.
        if matches!(
            *stage.peek(),
            Stage::Found(_) | Stage::Missed(_) | Stage::Refused(..)
        ) {
            stage.set(Stage::Blank);
        }
    };
    let icon = match &shown {
        Stage::Found(Offer {
            sign_in: SignIn::OAuth { .. },
            ..
        }) => Icon::Key,
        Stage::Added { .. } => Icon::Check,
        Stage::Found(_) | Stage::Refused(..) | Stage::Adding(_) => Icon::Plus,
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
                    button {
                        class: "mini",
                        r#type: "button",
                        onclick: move |_| super::close(shell),
                        "Close"
                        span { class: "k", "Esc" }
                    }
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
                    Below { shown: shown.clone(), typed: typed.clone(), signing: signing(), on_secret: move |value: String| {
                        secret.set(Password::new(value));
                    } }
                }
                div { class: "files-foot",
                    button {
                        class: "mini",
                        r#type: "button",
                        aria_label: "{dismiss}",
                        onclick: move |_| super::close(shell),
                        "{dismiss}"
                    }
                    button {
                        class: "mini primary",
                        r#type: "button",
                        aria_label: "{primary}",
                        disabled: !enabled,
                        onclick: press,
                        Glyph { icon }
                        "{primary}"
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
    on_secret: EventHandler<String>,
) -> Element {
    match shown {
        Stage::Blank => rsx! {
            p { class: "capnote", "{flow::before_looking(&typed, chrono::Utc::now())}" }
        },
        Stage::Looking => rsx! {
            p { class: "files-look acct-busy", role: "status",
                "Looking up the servers for {flow::domain_of(&typed).unwrap_or_default()}…"
            }
        },
        Stage::Missed(why) => rsx! {
            p { class: "capnote files-bad acct-said", role: "alert", "{why}" }
        },
        Stage::Found(offer) => rsx! {
            Found { offer: offer.clone() }
            Credential { offer, on_secret }
        },
        Stage::Refused(offer, why) => rsx! {
            Found { offer: offer.clone() }
            Credential { offer, on_secret }
            p { class: "capnote files-bad acct-said", role: "alert", "{why}" }
        },
        Stage::Adding(offer) => match (offer.sign_in, signing) {
            (SignIn::OAuth { .. }, Some(signing)) => rsx! {
                Found { offer }
                Browser { signing }
            },
            (sign_in, _) => {
                let waiting = match sign_in {
                    SignIn::Password => {
                        "Saving the account and putting the password in the system keyring…"
                            .to_owned()
                    }
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

/// The password field, or what a browser sign-in will do.
#[component]
fn Credential(offer: Offer, on_secret: EventHandler<String>) -> Element {
    match offer.sign_in {
        SignIn::Password => rsx! {
            span { class: "files-k", "Password" }
            Field {
                kind: FieldKind::Secret,
                value: String::new(),
                placeholder: format!("Password for {}", offer.address),
                extra: Some("acct-in".to_owned()),
                on_input: move |value: String| on_secret.call(value),
                on_focus: |_| {},
                on_blur: |_| {},
            }
            p { class: "capnote",
                "It goes to the system keyring, and to these servers when mailo signs in. It is never saved anywhere else."
            }
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
            button {
                class: "mini",
                r#type: "button",
                aria_label: "{copy_label}",
                onclick: move |_| copy(&url),
                "{copy_label}"
            }
        }
    }
}
