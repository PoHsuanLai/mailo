//! The add-account window: porter's sheet, drawn with quire's account parts, in a window of its
//! own.
//!
//! The window hosts the service. As it mounts it makes the two ends of a conversation
//! (`host::ends`), hands the service end to porter's account service over mailo's providers
//! (`host::run_mail`) and draws what the window end shows: each `SheetView` through
//! [`map::step_of`] into one of quire's parts, each thing the person does through [`map::acted`]
//! into the `SheetInput` the service is told. When the service ends the window closes; when the
//! person closes the window, the service's wait ends with it (its channel is gone) and so does a
//! browser sign-in that was waiting.
//!
//! What a finished add leaves for the rest of the app is the client's (the account) and the
//! window's notice: the current Space takes it in when it shows some accounts and not others.
//! Every window draws the new account as the client says the store moved.

use crate::ui::provider_chip::{MarkAt, style_of_mark};
use crate::ui::view::Marks;
use std::sync::Arc;

use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds_settings::use_environment;
use ds_shell::accounts::model::StepTitle;
use ds_shell::prelude::{
    BrowserWait, ProviderList, ReviewServices, ShowCode, SignInFailed, SignInForm, SignInWorking,
};
use mail_client::Client;
use mail_client::profile::ProfileId;
use porter_core::AccountId;

use super::Join;
use super::map::{self, Action, Out, Sheet, Step};
use super::size;
use crate::ui::app::Frame;
use crate::ui::space::{self, Spaces};
use crate::ui::style::STYLE;
use mail_ui_model::add_account::host::{self, Ended, Shown};
use mail_ui_model::add_account::provider::{Added, Seams};

/// Open an address in the system browser.
pub type Browse = dyn Fn(&str) -> Result<(), String> + Send + Sync;

/// What the window reaches the world with: the sign-in's seams, and the system's browser.
/// Compared by identity, as every other window's props are.
#[derive(Clone)]
pub struct Wiring(Arc<Reach>);

struct Reach {
    seams: Seams,
    browse: Arc<Browse>,
}

impl PartialEq for Wiring {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Wiring {
    /// The network, the keyring, the program's OAuth clients (`clients`) and the browser.
    pub(super) fn real(client: Client, clients: mail_client::prelude::OAuthClients) -> Wiring {
        Wiring::new(
            Seams::real(client, clients),
            Arc::new(|url| webbrowser::open(url).map_err(|why| why.to_string())),
        )
    }

    /// `seams` and `browse`, which a test makes.
    pub fn new(seams: Seams, browse: Arc<Browse>) -> Wiring {
        Wiring(Arc::new(Reach { seams, browse }))
    }
}

/// The window as quire opens it: the desktop's settings watched as the first window's root
/// watches them, then [`AddAccountShell`].
#[component]
pub(super) fn AddAccountWindow(wiring: Wiring, prefill: Option<String>, join: Join) -> Element {
    let desktop = crate::ui::launch::DesktopSettings::current();
    let spawner: Arc<dyn Spawner> = Arc::new(ds_blitz::TokioSpawner::current());
    let environment = use_environment(desktop.store(), desktop.prefs.clone(), spawner);
    use_context_provider(|| environment);
    crate::ui::prefs::use_window_settings();
    rsx! { AddAccountShell { wiring, prefill, join } }
}

/// What a test hands the window as a root context: how it signs in, and who is being signed in
/// again.
#[derive(Clone)]
pub struct Opened {
    wiring: Wiring,
    prefill: Option<String>,
    join: Join,
}

impl Opened {
    /// The window drawn for `wiring`, with `prefill` in its address when one is being signed in
    /// again; an account added in it joins the Space on screen.
    pub fn new(wiring: Wiring, prefill: Option<String>) -> Opened {
        Opened {
            wiring,
            prefill,
            join: Join::OnScreen,
        }
    }
}

/// The window as a test drives it: [`AddAccountShell`] over the [`Opened`] its root context
/// names, without the watch on the settings directory.
pub fn add_account_root() -> Element {
    let Opened {
        wiring,
        prefill,
        join,
    } = consume_context::<Opened>();
    rsx! { AddAccountShell { wiring, prefill, join } }
}

#[component]
fn AddAccountShell(wiring: Wiring, prefill: Option<String>, join: Join) -> Element {
    // The provider icons as the first window read them, fetched here too when any is missing:
    // the list offers every known provider before an account is on one.
    let loaded = try_consume_context::<mail_client::provider::icon::Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    crate::ui::provider_chip::use_fetch_missing(icons);
    crate::ui::host::use_window_host();
    let dirs = try_consume_context::<crate::ui::appearance::WindowDirs>();
    let _ = crate::ui::prefs::use_prefs(dirs.as_ref());
    // Icons or letters, as the provider-marks setting says.
    let marks = Marks::from(
        crate::ui::prefs::use_settings()
            .read()
            .window
            .provider_marks,
    );
    // The Spaces as the first window keeps them: this window wears the Space and may add to it.
    let mut spaces = use_signal({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map(space::load)
                .or_else(try_consume_context::<Spaces>)
                .unwrap_or_else(|| crate::ui::space::first_run(&[]))
        }
    });
    let mut sheet = use_signal(Sheet::default);
    let mut finished = use_signal(|| None::<Ended>);

    // The conversation: the service runs on the one end, the window draws from the other.
    let (end, added) = use_hook({
        let wiring = wiring.clone();
        move || {
            let (end, sheets) = host::ends();
            let added = Added::default();
            let (reach, said, typed) = (Arc::clone(&wiring.0), added.clone(), prefill);
            spawn(async move {
                finished.set(Some(
                    host::run_mail(&reach.seams, &said, typed, sheets).await,
                ));
            });
            (end, added)
        }
    });

    // What the service shows, as it shows it.
    use_future({
        let end = end.clone();
        let browse = Arc::clone(&wiring.0.browse);
        move || {
            let mut views = end.views.clone();
            let inputs = end.inputs.clone();
            let browse = Arc::clone(&browse);
            async move {
                while views.changed().await.is_ok() {
                    let now = views.borrow_and_update().clone();
                    let Shown::View(view) = now else {
                        continue;
                    };
                    let (next, out) = map::shown(sheet.peek().clone(), *view);
                    // A review with nothing on it to choose or to read (mailo adds an account's
                    // mail and nothing else of what a provider offers) is not drawn: what the
                    // person typed and sent was the confirmation, and the account is added at
                    // once. One that says what the provider lacks is still shown.
                    if matches!(map::step_of(&next), Some(Step::Review(props)) if props.services.is_empty())
                    {
                        let (confirming, input) = map::acted(next, Action::Confirm);
                        sheet.set(confirming);
                        if let Some(input) = input {
                            // The service has ended if nobody is receiving, and the window closes
                            // with it.
                            let _ = inputs.send(input);
                        }
                        continue;
                    }
                    sheet.set(next);
                    for Out::OpenPage(page) in out {
                        // The address is on screen too, to copy, when no browser takes it.
                        let _ = browse(page.as_str());
                    }
                }
            }
        }
    });

    // What the person does, as what the service is told.
    let send = {
        let inputs = end.inputs.clone();
        Callback::new(move |action: Action| {
            let (next, input) = map::acted(sheet.peek().clone(), action);
            sheet.set(next);
            if let Some(input) = input {
                // The service has ended if nobody is receiving, and the window closes with it.
                let _ = inputs.send(input);
            }
        })
    };

    // The service ended: an account added is the app's to draw, and the window is done.
    let handle = ds_blitz::use_window_handle();
    let added_to = added.clone();
    use_effect(move || {
        let Some(ended) = finished() else { return };
        // Nothing is left to draw (and nothing typed is kept) while the window goes.
        sheet.set(Sheet::default());
        if ended == Ended::Added
            && let Some(account) = added_to.take().and_then(|address| account_at(&address))
        {
            into_scope(&mut spaces, account, join);
        }
        if let Some(handle) = &handle {
            handle.close();
        }
    });

    let step = map::step_of(&sheet.read()).map(|step| step.said(added.said()));
    // The window is as tall as the step it shows, measured after each layout, at the step's
    // width (`size`).
    let fit = ds_blitz::use_content_fit(size::width(step.as_ref()), size::least());
    let slug = step.as_ref().map_or("none", Step::slug);
    rsx! {
        Frame { spaces, sheet: Some(ds_shell::stylesheet()),
            style { {STYLE} }
            div {
                class: "add-account-window",
                "data-step": slug,
                tabindex: "0",
                onkeydown: move |event| {
                    if event.key() == Key::Escape {
                        send.call(Action::Cancel);
                    }
                },
                if let Some(step) = step {
                    // One column per step: a new step mounts afresh and the keyboard lands on its
                    // default control again.
                    div {
                        class: "add-account-step",
                        key: "{slug}",
                        onmounted: move |event| fit.on_mounted(event),
                        {body(step, send, marks)}
                    }
                }
            }
        }
    }
}

/// One step's column, with every event sent as an [`Action`].
/// One step as quire's part, its provider marks drawn as `marks` says.
fn body(step: Step, send: Callback<Action>, marks: Marks) -> Element {
    let cancel = move |()| send.call(Action::Cancel);
    let back = move |()| send.call(Action::Back);
    let copy = move |text: String| {
        crate::ui::hover::copy(&text);
        send.call(Action::Copied);
    };
    let act = move |action: Option<Action>| {
        if let Some(action) = action {
            send.call(action);
        }
    };
    match step {
        Step::Providers(props) => rsx! {
            ProviderList {
                providers: props
                    .providers
                    .into_iter()
                    .map(|entry| {
                        let style = style_of_mark(entry.mark, marks, MarkAt::List);
                        entry.styled(style)
                    })
                    .collect::<Vec<_>>(),
                query: props.query,
                cursor: props.cursor,
                on_query: move |query| send.call(Action::Query(query)),
                on_cursor: move |pick| act(map::list_key(&pick).map(Action::Cursor)),
                on_pick: move |pick| act(map::list_key(&pick).map(Action::Pick)),
                on_cancel: cancel,
                title: StepTitle::Own,
            }
        },
        Step::SignIn(props) => rsx! {
            SignInForm {
                provider: props.provider,
                mark: props.mark,
                style: style_of_mark(props.mark, marks, MarkAt::Header),
                fields: props.fields,
                problem: props.problem,
                on_input: move |(role, text)| send.call(map::typed(role, text)),
                on_submit: move |()| send.call(Action::Submit),
                on_back: back,
                on_cancel: cancel,
                title: StepTitle::Own,
            }
        },
        Step::Browser {
            provider,
            mark,
            url,
            copied,
        } => rsx! {
            BrowserWait {
                provider,
                mark: Some(mark),
                style: style_of_mark(mark, marks, MarkAt::Header),
                url,
                copied,
                on_open_again: move |()| send.call(Action::OpenAgain),
                on_copy: copy,
                on_cancel: cancel,
                title: StepTitle::Own,
            }
        },
        Step::Code {
            provider,
            code,
            url,
            copied,
        } => rsx! {
            ShowCode {
                provider,
                code,
                url,
                copied,
                on_copy: copy,
                on_cancel: cancel,
                title: StepTitle::Own,
            }
        },
        Step::Review(props) => rsx! {
            ReviewServices {
                account: props.account,
                services: props.services,
                allow: props.allow,
                // No switch is drawn (see `map::step_of`), so none is heard.
                on_toggle: move |_| {},
                on_done: move |()| send.call(Action::Confirm),
                on_back: back,
                on_cancel: cancel,
                title: StepTitle::Own,
            }
        },
        Step::Working { provider, mark } => rsx! {
            SignInWorking {
                provider,
                mark: Some(mark),
                style: style_of_mark(mark, marks, MarkAt::Header),
                on_cancel: cancel,
                title: StepTitle::Own,
            }
        },
        Step::Failed {
            provider,
            why,
            detail,
            recovery,
        } => rsx! {
            SignInFailed {
                provider,
                why,
                recovery,
                on_retry: move |()| send.call(Action::Retry),
                on_back: back,
                on_cancel: cancel,
                title: StepTitle::Own,
            }
            if let Some(detail) = detail {
                // What the sentence leaves out: the server's own words, under the buttons.
                p { class: "add-account-detail", role: "alert", "{detail}" }
            }
        },
    }
}

/// The account mailo's client has for `address`, which the add has just written.
pub(super) fn account_at(address: &str) -> Option<AccountId> {
    let client = crate::ui::live::client();
    crate::ui::data::account_rows(&client)
        .into_iter()
        .find(|row| row.address == address)
        .map(|row| row.id)
}

/// A new account joins the Space it was added from, as `join` names it; or none.
pub(super) fn into_scope(spaces: &mut Signal<Spaces>, account: AccountId, join: Join) {
    if join == Join::Nowhere {
        return;
    }
    // The Spaces as they are on disk: this window's copy may be older than a Space another
    // window added or switched to meanwhile, and keeping it would undo that.
    if let Some(dirs) = try_consume_context::<crate::ui::appearance::WindowDirs>()
        && let Some(stored) = space::stored(&dirs)
        && *spaces.peek() != stored
    {
        spaces.set(stored);
    }
    let widened = {
        let mut all = spaces.write();
        let into = match join {
            Join::Into(space) => space,
            Join::OnScreen | Join::Nowhere => all.current().id,
        };
        let mut widened = false;
        all.edit(into, |space| {
            widened = space.payload.widen(&ProfileId::default(), account)
        });
        widened
    };
    if widened {
        crate::ui::frame::keep(&spaces.read());
    }
}

// By hand: the wiring holds closures.
impl std::fmt::Debug for Wiring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wiring")
    }
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("prefill", &self.prefill)
            .finish_non_exhaustive()
    }
}
