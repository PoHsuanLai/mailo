//! Hosting porter's account service in this process, with the add-account window as the host
//! that draws its sheet.
//!
//! `porter_service::Sheets` is the seam between the service (which runs the sheet's machine and
//! the provider's sign-in) and whoever draws: here a pair of channels. The service end, a
//! [`WindowSheets`], is handed the views and reads the person's inputs; the window's end
//! ([`Ends::window`]) is what the window draws from and answers into. Nothing crosses them but
//! porter's values: a typed password travels as `SheetInput::Submit` and is gone from the window
//! the moment it is sent.

use std::sync::{Arc, Mutex};

use porter_core::consent::{ConsentAnswer, ConsentAsk};
use porter_core::sheet::{SheetInput, SheetView};
use porter_core::wire::{ParentWindow, ProviderHint};
use porter_core::{AccountsReply, AccountsRequest, AppId, UnixSeconds};
use porter_provider::Provider;
use porter_secrets::{Secrets, SecretsError};
use porter_service::{AccountService, Clock, Registry, SheetFault, SheetLink, SheetOpen, Sheets};
use tokio::sync::{mpsc, watch};

use super::provider::{Added, Seams, offered};

/// What the window is to draw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum Shown {
    /// The service has not opened the sheet yet.
    #[default]
    Nothing,
    /// This. Told again with the same view, it is a view shown again (the browser's page, on
    /// "Open Again").
    View(SheetView),
    /// The sheet is closed: the service has finished.
    Closed,
}

/// The window's end: the views to draw and where its answers go.
#[derive(Debug, Clone)]
pub(super) struct WindowEnd {
    pub views: watch::Receiver<Shown>,
    pub inputs: mpsc::UnboundedSender<SheetInput>,
}

/// The service's end, as `porter_service::Sheets`.
#[derive(Debug)]
pub(super) struct WindowSheets {
    views: Arc<watch::Sender<Shown>>,
    inputs: Mutex<Option<mpsc::UnboundedReceiver<SheetInput>>>,
}

/// The two ends of one conversation.
pub(super) fn ends() -> (WindowEnd, WindowSheets) {
    let (views, seen) = watch::channel(Shown::Nothing);
    let (answers, inputs) = mpsc::unbounded_channel();
    (
        WindowEnd {
            views: seen,
            inputs: answers,
        },
        WindowSheets {
            views: Arc::new(views),
            inputs: Mutex::new(Some(inputs)),
        },
    )
}

/// An open conversation: what is shown, and what the person did.
#[derive(Debug)]
pub(super) struct WindowLink {
    views: Arc<watch::Sender<Shown>>,
    inputs: mpsc::UnboundedReceiver<SheetInput>,
}

impl SheetLink for WindowLink {
    async fn update(&mut self, view: SheetView) -> Result<(), SheetFault> {
        self.views
            .send(Shown::View(view))
            .map_err(|_| SheetFault::Closed)
    }

    async fn input(&mut self) -> Result<SheetInput, SheetFault> {
        self.inputs.recv().await.ok_or(SheetFault::Closed)
    }
}

// Dropping the link ends the sheet: the window closes with it.
impl Drop for WindowLink {
    fn drop(&mut self) {
        self.views.send_replace(Shown::Closed);
    }
}

impl Sheets for WindowSheets {
    type Link = WindowLink;

    async fn consent(&self, _ask: ConsentAsk, _window: &ParentWindow) -> ConsentAnswer {
        // mailo asks for no one's consent: it is the only caller of its own service.
        ConsentAnswer::Dismissed
    }

    async fn conversation(&self, open: SheetOpen) -> Result<WindowLink, SheetFault> {
        let inputs = self
            .inputs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or(SheetFault::Unavailable)?;
        self.views.send_replace(Shown::View(open.view));
        Ok(WindowLink {
            views: Arc::clone(&self.views),
            inputs,
        })
    }
}

/// The account service has no secrets of its own to keep: the provider files nothing with it
/// (mailo keeps its accounts' secrets itself, `mail_runtime::AccountSecrets`).
#[derive(Debug, Clone, Copy)]
pub(super) struct NoSecrets;

impl Secrets for NoSecrets {
    async fn put(
        &self,
        _key: &porter_core::SecretKey,
        _value: &porter_core::Credential,
    ) -> Result<(), SecretsError> {
        Err(SecretsError::Unavailable)
    }

    async fn get(
        &self,
        _key: &porter_core::SecretKey,
    ) -> Result<porter_core::Credential, SecretsError> {
        Err(SecretsError::Missing)
    }

    async fn delete(&self, _key: &porter_core::SecretKey) -> Result<(), SecretsError> {
        Ok(())
    }

    async fn delete_account(&self, _account: &porter_core::AccountId) -> Result<(), SecretsError> {
        Ok(())
    }
}

/// The system's clock.
#[derive(Debug, Clone, Copy)]
pub(super) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UnixSeconds {
        UnixSeconds(chrono::Utc::now().timestamp())
    }
}

/// Who asks the service: this app, hosting it itself.
fn caller() -> AppId {
    AppId {
        name: porter_core::AppName::parse("org.quire.Mail")
            .unwrap_or_else(|_| unreachable!("a reverse-DNS name")),
        isolation: porter_core::Isolation::InProcess,
    }
}

/// How an add ended, as the window needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ended {
    /// An account was added.
    Added,
    /// The person closed the sheet, or it ended without one.
    NoAccount,
}

/// Run the add-account conversation to its end: the service over `providers`, the sheet through
/// `sheets`. Resolves when the sheet closes.
pub(super) async fn run<P: Provider>(providers: Vec<P>, sheets: WindowSheets) -> Ended {
    let service = AccountService::new(
        providers,
        Registry::from_persisted(porter_core::store::Persisted::empty()),
        NoSecrets,
        sheets,
        SystemClock,
    );
    let request = AccountsRequest::AddAccount {
        hint: ProviderHint::Any,
        window: ParentWindow::Unparented,
    };
    match service.handle(&caller(), request).await {
        AccountsReply::Added(_) => Ended::Added,
        _ => Ended::NoAccount,
    }
}

/// [`run`] for mail: the providers mailo offers, over `seams`, with the address of the account the
/// person is signing in again already typed, if one is. `added` is told what was added.
pub(super) async fn run_mail(
    seams: &Seams,
    added: &Added,
    prefill: Option<String>,
    sheets: WindowSheets,
) -> Ended {
    run(offered(seams, added, prefill), sheets).await
}
