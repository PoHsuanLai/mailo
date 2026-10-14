//! Signing in straight in the browser, from the main window, with no window of mailo's own.
//!
//! On the web, "Sign in with Google" goes to Google's page and comes back signed in. A provider
//! whose accounts sign in with an OAuth redirect needs nothing more here either, so Add Account in
//! the Space's menu and in Settings is a small menu: each such provider by name
//! (which of them, [`entry`] decides from its provider file and whether a client is at hand),
//! then Other Account…, which opens the add-account window for every provider that needs a form.
//!
//! The sign-in itself is `mail_core::account::sign_in_with`. What is here is its one-at-a-time
//! state ([`Phase`], shared by every window on the event loop's thread), the quiet row at the top
//! of the sidebar while it waits ([`PendingSignIn`]), and what the main window says when it ends
//! ([`DirectSignInWatch`]): a toast, the shared revision moved so every window draws the account,
//! and the current Space taking it in, as the add-account window does.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::progress::model::{Progress, ProgressStyle};
use ds::components::controls::progress::view::ProgressIndicator;
use ds::host::measure::MountedRef;
use ds::motion::detail::operation::{Operation, PendingToken};
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::account::direct::{WAIT, name_of, signs_in_directly};
use mail_core::account::{Mode, SignInError};
use mail_core::{OAuthClients, SqliteStore};
use mail_domain::AuthPlan;
use porter_core::AuthKind;
use porter_provider::{Issuer, ProviderSpec};
use tokio::sync::watch;

use super::Ask;
use crate::ui::motion::Follow;
use crate::ui::space::Spaces;

/// Where an add-account entry goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Entry {
    /// Straight to the issuer's page in the browser.
    Redirect(Issuer),
    /// The add-account window, for a provider that needs a form (a password, an app password,
    /// servers typed), or one whose sign-in cannot start here.
    Window,
}

/// Where `spec`'s accounts are added: in the browser when they sign in with an OAuth redirect,
/// Mail can add them from the sign-in alone and `has_client` says the issuer's client is at hand;
/// in the window otherwise, whose list explains a missing client.
pub(in crate::ui) fn entry(spec: &ProviderSpec, has_client: &dyn Fn(Issuer) -> bool) -> Entry {
    match (&spec.auth.kind, spec.auth.issuer) {
        (AuthKind::OAuthPkce, Some(issuer)) => redirect_if(issuer, has_client),
        _ => Entry::Window,
    }
}

/// Where an account already here goes to sign in again: by the same rule, through the plan it
/// was added with.
pub(in crate::ui) fn entry_for_plan(auth: &AuthPlan, has_client: &dyn Fn(Issuer) -> bool) -> Entry {
    match auth {
        AuthPlan::OAuth { issuer, .. } => redirect_if(*issuer, has_client),
        AuthPlan::Password { .. } => Entry::Window,
    }
}

fn redirect_if(issuer: Issuer, has_client: &dyn Fn(Issuer) -> bool) -> Entry {
    if signs_in_directly(issuer) && has_client(issuer) {
        Entry::Redirect(issuer)
    } else {
        Entry::Window
    }
}

/// The providers mailo offers that go straight to the browser, by name, in the list's order.
pub(in crate::ui) fn redirects(
    specs: &[ProviderSpec],
    has_client: &dyn Fn(Issuer) -> bool,
) -> Vec<(Issuer, String)> {
    specs
        .iter()
        .filter_map(|spec| match entry(spec, has_client) {
            Entry::Redirect(issuer) => Some((issuer, spec.label.clone())),
            Entry::Window => None,
        })
        .collect()
}

/// What a row of the add menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum AddPick {
    /// Sign in with this issuer in the browser.
    Redirect(Issuer),
    /// The add-account window.
    Other,
}

/// The add menu's rows: each provider that goes straight to the browser, then Other Account….
pub(in crate::ui) fn add_menu(redirects: &[(Issuer, String)]) -> Vec<MenuItem<AddPick>> {
    let mut rows: Vec<MenuItem<AddPick>> = redirects
        .iter()
        .map(|(issuer, label)| MenuItem::new(AddPick::Redirect(*issuer), label.clone()))
        .collect();
    if !rows.is_empty() {
        rows.push(MenuItem::Separator);
    }
    rows.push(MenuItem::new(AddPick::Other, "Other Account\u{2026}"));
    rows
}

/// A direct sign-in, as the world is reached for one: a root context. The launched program's
/// root hands the real one ([`DirectSignIns::real`]); a test hands a recorder, or none, and a
/// window without one offers no provider by name and opens the add-account window instead, so a
/// test's window can never reach a browser or an issuer.
#[derive(Clone)]
pub struct DirectSignIns(pub Arc<dyn SignInDirectly>);

impl DirectSignIns {
    /// The program's: `clients`, the system browser, the keyring and `store`. For the launched
    /// window's root alone.
    pub(in crate::ui) fn real(store: Arc<SqliteStore>, clients: OAuthClients) -> DirectSignIns {
        DirectSignIns(Arc::new(Real { store, clients }))
    }
}

/// A sign-in under way: the address added, or why not. Dropping it cancels the sign-in and frees
/// its listener.
pub type Signing = Pin<Box<dyn Future<Output = Result<String, SignInError>> + Send>>;

/// How a direct sign-in reaches the world: the program's OAuth clients, the browser, the keyring
/// and the store in the program, a recorder in a test.
pub trait SignInDirectly: Send + Sync + 'static {
    /// Whether an OAuth client for `issuer` is at hand.
    fn has_client(&self, issuer: Issuer) -> bool;
    /// Open `issuer`'s page and wait for it: `mode`'s account added or signed in again.
    fn sign_in(&self, issuer: Issuer, mode: Mode) -> Signing;
}

impl std::fmt::Debug for DirectSignIns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirectSignIns")
    }
}

/// The program's: its OAuth clients, the system browser, the keyring and `store`.
struct Real {
    store: Arc<SqliteStore>,
    clients: OAuthClients,
}

impl SignInDirectly for Real {
    fn has_client(&self, issuer: Issuer) -> bool {
        super::provider::has_client(issuer, &self.clients)
    }

    fn sign_in(&self, issuer: Issuer, mode: Mode) -> Signing {
        let store = Arc::clone(&self.store);
        let clients = self.clients.clone();
        Box::pin(async move {
            // `sign_in_with` borrows the browser's callback across its wait, which is not `Send`,
            // so it runs on a thread of its own, as the add-account sheet's sign-in does, and
            // this future waits for it. Dropping the future drops `stop`, which ends the sign-in
            // and frees its listener.
            let (done, answer) = tokio::sync::oneshot::channel();
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let spawned = std::thread::Builder::new()
                .name("mailo-sign-in".to_owned())
                .spawn(move || {
                    let ended = crate::edge::block_on(async {
                        let saved = clients.saved();
                        let environment = crate::edge::environment();
                        let secrets = crate::edge::secrets();
                        let browse = |url: &str| webbrowser::open(url);
                        let profiles = mail_core::account::direct::Profiles::default();
                        mail_core::account::sign_in_with(
                            issuer,
                            mode,
                            mail_core::account::Direct {
                                store: &store,
                                secrets: secrets.as_ref(),
                                saved: &saved,
                                environment: &environment,
                                browse: &browse,
                                profiles: &profiles,
                                wait: WAIT,
                                now: Utc::now(),
                            },
                            async {
                                let _ = stopped.await;
                            },
                        )
                        .await
                        .map(|added| added.address)
                    });
                    let _ = done.send(ended);
                });
            if let Err(why) = spawned {
                return Err(SignInError::Network(why.to_string()));
            }
            let _stop = stop;
            answer.await.unwrap_or(Err(SignInError::Cancelled))
        })
    }
}

/// The direct sign-in this window may start: its root's, and none when the root handed none.
/// Never a real one made up here: only the launched program's root has the real one, so a test's
/// window that forgot to hand a recorder reaches nothing.
fn runner() -> Option<DirectSignIns> {
    try_consume_context::<DirectSignIns>()
}

/// The providers the add menu lists by name now. Empty when no direct sign-in can start here,
/// and then "+" opens the window as it always did.
pub(in crate::ui) fn offered_redirects() -> Vec<(Issuer, String)> {
    match runner() {
        Some(runner) => redirects(&super::provider::offered_specs(), &|issuer| {
            runner.0.has_client(issuer)
        }),
        None => Vec::new(),
    }
}

/// What a direct sign-in is doing, for every window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::ui) enum Phase {
    #[default]
    Idle,
    /// The browser is open, and the main window waits for it. `raised` counts the picks made
    /// while it waits, each of which brings its row back to the person's eye.
    Waiting {
        run: u64,
        issuer: Issuer,
        mode: Mode,
        raised: u32,
    },
    /// It ended: the address added, or why not.
    Ended {
        run: u64,
        issuer: Issuer,
        mode: Mode,
        outcome: Result<String, SignInError>,
    },
}

thread_local! {
    /// The one direct sign-in, as every window sees it. Windows share the event loop's thread.
    static PHASE: watch::Sender<Phase> = watch::channel(Phase::Idle).0;
    /// The number of the last sign-in started.
    static RUNS: Cell<u64> = const { Cell::new(0) };
    /// Whether this thread's main window is watching ([`DirectSignInWatch`]), and so whether a
    /// sign-in started here will be run and heard.
    static WATCHED: RefCell<u32> = const { RefCell::new(0) };
}

fn phase_now() -> Phase {
    PHASE.with(|phase| phase.borrow().clone())
}

/// Sign in with `issuer` in the browser: `mode`'s account added, or signed in again. Call it from
/// an event handler, in any window.
///
/// One at a time: a pick while one waits brings its row back instead of opening a second page.
/// With no client for the issuer, or nothing here to run a direct sign-in, the window opens
/// instead, and says why.
pub(in crate::ui) fn start(issuer: Issuer, mode: Mode) {
    if matches!(phase_now(), Phase::Waiting { .. }) {
        PHASE.with(|phase| {
            phase.send_modify(|now| {
                if let Phase::Waiting { raised, .. } = now {
                    *raised += 1;
                }
            });
        });
        return;
    }
    let ready = runner().is_some_and(|runner| runner.0.has_client(issuer))
        && WATCHED.with(|watched| *watched.borrow() > 0);
    if !ready {
        super::request(window_ask(&mode));
        return;
    }
    let run = RUNS.with(|runs| {
        runs.set(runs.get() + 1);
        runs.get()
    });
    PHASE.with(|phase| {
        let _ = phase.send_replace(Phase::Waiting {
            run,
            issuer,
            mode,
            raised: 0,
        });
    });
}

/// Stop the sign-in that waits. Its page stays open in the browser, and coming back to it finds
/// nobody listening.
pub(in crate::ui) fn cancel() {
    end_with(None, Err(SignInError::Cancelled));
}

/// End the waiting sign-in (`run`'s, when named) with `outcome`. A sign-in that has already ended
/// is left as it ended.
fn end_with(run: Option<u64>, outcome: Result<String, SignInError>) {
    PHASE.with(|phase| {
        let _ = phase.send_if_modified(|now| {
            let Phase::Waiting {
                run: waiting,
                issuer,
                mode,
                ..
            } = now
            else {
                return false;
            };
            if run.is_some_and(|run| run != *waiting) {
                return false;
            }
            let ended = Phase::Ended {
                run: *waiting,
                issuer: *issuer,
                mode: mode.clone(),
                outcome,
            };
            *now = ended;
            true
        });
    });
}

/// What the window opens with for `mode`: the address of an account being signed in again.
fn window_ask(mode: &Mode) -> Ask {
    match mode {
        Mode::Add => Ask::default(),
        Mode::Again { address } => Ask {
            address: Some(address.clone()),
        },
    }
}

/// What a toast's button does after a sign-in that did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Retry {
    /// The same direct sign-in again.
    Direct { issuer: Issuer, mode: Mode },
    /// The add-account window, which says what is missing.
    Window(Ask),
}

impl Retry {
    /// The button's words.
    pub(in crate::ui) fn label(&self) -> &'static str {
        match self {
            Retry::Direct { .. } => "Try Again",
            Retry::Window(_) => "Use Add Account\u{2026}",
        }
    }
}

/// Do what a toast's button offered. Call it from an event handler.
pub(in crate::ui) fn retry(retry: Retry) {
    match retry {
        Retry::Direct { issuer, mode } => start(issuer, mode),
        Retry::Window(ask) => super::request(ask),
    }
}

/// What the toast says when a sign-in ends, and what its button offers.
pub(in crate::ui) fn told(
    issuer: Issuer,
    mode: &Mode,
    outcome: &Result<String, SignInError>,
) -> (String, Follow) {
    match outcome {
        Ok(address) => {
            let text = match mode {
                Mode::Add => format!("Added {address}"),
                Mode::Again { .. } => format!("Signed in to {address}"),
            };
            (text, Follow::Nothing)
        }
        Err(SignInError::Declined | SignInError::Cancelled) => {
            ("Sign-in cancelled".to_owned(), Follow::Nothing)
        }
        // Nothing to try again here: the window explains what is missing and offers the rest.
        Err(why @ (SignInError::NoClient(_) | SignInError::NotMail(_))) => (
            why.to_string(),
            Follow::SignIn(Retry::Window(window_ask(mode))),
        ),
        Err(why) => (
            why.to_string(),
            Follow::SignIn(Retry::Direct {
                issuer,
                mode: mode.clone(),
            }),
        ),
    }
}

/// The direct sign-in's phase, as a signal of this window's.
pub(in crate::ui) fn use_phase() -> Signal<Phase> {
    let mut phase = use_signal(phase_now);
    use_future(move || async move {
        let mut heard = PHASE.with(|phase| phase.subscribe());
        loop {
            let now = heard.borrow_and_update().clone();
            if *phase.peek() != now {
                phase.set(now);
            }
            if heard.changed().await.is_err() {
                return;
            }
        }
    });
    phase
}

/// The main window's side of a direct sign-in: it runs the one that is asked for, and says how
/// it ended. Mounted once, at the main window's root, for as long as the window is open; draws
/// nothing.
#[component]
pub(in crate::ui) fn DirectSignInWatch(spaces: Signal<Spaces>) -> Element {
    use_hook(|| WATCHED.with(|watched| *watched.borrow_mut() += 1));
    use_drop(|| WATCHED.with(|watched| *watched.borrow_mut() -= 1));
    let phase = use_phase();
    let runner = use_hook(runner);
    // A sign-in that ended before this window opened is not this window's to say.
    let before = use_hook(|| match phase_now() {
        Phase::Ended { run, .. } => run,
        Phase::Idle | Phase::Waiting { .. } => 0,
    });
    let started = use_hook(|| CopyValue::new(before));
    let heard = use_hook(|| CopyValue::new(before));
    use_effect(move || {
        let now = phase();
        match now {
            Phase::Waiting {
                run, issuer, mode, ..
            } if run > *started.peek() => {
                let mut started = started;
                started.set(run);
                match runner.clone() {
                    Some(runner) => {
                        spawn(drive(runner, run, issuer, mode));
                    }
                    None => end_with(Some(run), Err(SignInError::NoClient(issuer))),
                }
            }
            Phase::Ended {
                run,
                issuer,
                mode,
                outcome,
            } if run > *heard.peek() => {
                let mut heard = heard;
                heard.set(run);
                settle(spaces, issuer, &mode, &outcome);
            }
            _ => {}
        }
    });
    rsx! {}
}

/// Run sign-in `run` until it ends, or until something else ends it (Cancel).
async fn drive(runner: DirectSignIns, run: u64, issuer: Issuer, mode: Mode) {
    let signing = runner.0.sign_in(issuer, mode);
    let mut heard = PHASE.with(|phase| phase.subscribe());
    let outcome = tokio::select! {
        outcome = signing => outcome,
        // Ended here (Cancel): whoever ended it said how. Dropping `signing` stops it.
        () = left(&mut heard, run) => return,
    };
    end_with(Some(run), outcome);
}

/// Resolves once sign-in `run` is no longer the one waiting.
async fn left(heard: &mut watch::Receiver<Phase>, run: u64) {
    loop {
        if !matches!(&*heard.borrow_and_update(), Phase::Waiting { run: now, .. } if *now == run) {
            return;
        }
        if heard.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

/// Say how sign-in ended, and draw what it added: the shared revision moves so every window reads
/// the store again, and the current Space takes the account in when it shows only some.
fn settle(
    mut spaces: Signal<Spaces>,
    issuer: Issuer,
    mode: &Mode,
    outcome: &Result<String, SignInError>,
) {
    if let Ok(address) = outcome {
        if let Some(account) = super::window::account_at(address) {
            super::window::into_scope(&mut spaces, account);
        }
        if let Some(revisions) = try_consume_context::<crate::ui::revisions::Revisions>() {
            revisions.bump();
        }
    }
    let (text, follow) = told(issuer, mode, outcome);
    crate::ui::motion::tell(text, follow);
}

/// The words of the row while `issuer`'s sign-in waits.
pub(in crate::ui) fn waiting_words(issuer: Issuer) -> String {
    format!("Signing in with {}\u{2026}", name_of(issuer))
}

/// The quiet row at the top of the sidebar while a direct sign-in waits: a spinner, what it waits
/// for, and a button that stops it. Draws nothing otherwise.
#[component]
pub(in crate::ui) fn PendingSignIn() -> Element {
    let phase = use_phase();
    let Phase::Waiting { issuer, raised, .. } = phase() else {
        return rsx! {};
    };
    let words = waiting_words(issuer);
    rsx! {
        // Keyed by the picks made while it waits: each one draws the row afresh, so it comes in
        // again where the person is looking.
        div { key: "{raised}", class: "direct-sign-in", role: "status",
            Spinning {}
            div { class: "direct-sign-in-words",
                Label { text: words, style: LabelStyle::Footnote }
            }
            Button {
                size: ControlSize::Small,
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                icon: Icon::X,
                label: "Cancel sign-in",
                title: Some("Stop waiting for the browser".to_owned()),
                onclick: crate::ui::press::on_primary(cancel),
            }
        }
    }
}

/// quire's small spinner, for as long as it shows.
#[component]
fn Spinning() -> Element {
    let token = use_hook(PendingToken::start);
    rsx! {
        ProgressIndicator {
            style: ProgressStyle::Spinner,
            progress: Progress::Unknown(Operation::Running(token)),
            size: ControlSize::Small,
        }
    }
}

/// The add menu, hung from `anchor`: each provider that goes straight to the browser, then Other
/// Account…, and a pick does what it says.
#[component]
pub(in crate::ui) fn AddMenu(
    anchor: Option<MountedRef>,
    redirects: Vec<(Issuer, String)>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        Menu::<AddPick> {
            placement: MenuPlacement::default(),
            anchor: crate::ui::menu::anchor_for(anchor, None),
            items: add_menu(&redirects),
            onpick: move |pick: AddPick| {
                picked(pick);
                on_close.call(());
            },
            onclose: on_close,
        }
    }
}

/// Do what a row of the add menu says. Call it from an event handler.
pub(in crate::ui) fn picked(pick: AddPick) {
    match pick {
        AddPick::Redirect(issuer) => start(issuer, Mode::Add),
        AddPick::Other => super::open(),
    }
}

/// The add menu's way in, for a control that adds an account: the menu when some provider goes
/// straight to the browser (`open_menu` is told them), else the window.
pub(in crate::ui) fn add_from(mut open_menu: impl FnMut(Vec<(Issuer, String)>)) {
    let offered = offered_redirects();
    if offered.is_empty() {
        super::open();
    } else {
        open_menu(offered);
    }
}

/// Sign `address`'s account in again: straight in the browser when its plan signs in with a
/// redirect whose client is at hand, else in the window with the address typed. Call it from an
/// event handler.
pub(in crate::ui) fn again(address: String) {
    let store = consume_context::<Arc<SqliteStore>>();
    let plan = crate::ui::data::account_rows(&store)
        .into_iter()
        .find(|row| row.address == address)
        .map(|row| row.plan.auth);
    let entry = match (plan, runner()) {
        (Some(auth), Some(runner)) => entry_for_plan(&auth, &|issuer| runner.0.has_client(issuer)),
        _ => Entry::Window,
    };
    match entry {
        Entry::Redirect(issuer) => start(issuer, Mode::Again { address }),
        Entry::Window => super::open_for(address),
    }
}
