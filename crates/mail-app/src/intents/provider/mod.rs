//! Mailo as an intent provider: what the router asks of it, answered over the store.
//!
//! [`Provider`] is the typed side of `org.quire.IntentProvider1` and knows nothing of the bus:
//! `serve` decodes the router's JSON, calls it and encodes what it returns, so everything here
//! runs in a test against a store and nothing else.

mod act;
mod contacts;
mod drafts;
mod find;
mod threads;
mod token;

use super::APP;
use super::wire::{
    AppRefusal, Context, EntityId, EntityRef, Hit, Invocation, Label, Labelled, Outcome, Output,
    Preview, Privacy, SuggestAsk, UndoFault, Undoable, Visible,
};
use act::Act;
use mail_core::undo::UndoStack;
use mail_domain::ThreadId;
use mail_runtime::SigningStore;
use mail_store::SqliteStore;
use std::sync::{Arc, Mutex};
use token::Token;

/// Mailo's side of the router's calls.
pub struct Provider {
    store: Arc<SqliteStore>,
    /// For a message that is signed or encrypted when it is sent: the keys' passphrases and
    /// the credentials the keyring holds.
    secrets: Arc<dyn SigningStore>,
    /// What conversation actions did, newest last: the undo tokens name entries of it. In
    /// memory, so a token outlives neither this process nor the window's own Cmd+Z stack, which
    /// is another process's.
    stack: Mutex<UndoStack>,
    /// Shows a conversation in mailo's window: [`Opener::window`], or a test's stand-in.
    opener: Opener,
}

/// What `mail.thread.open` does with the conversation it is given and the activation token that
/// came with the call: show it in a window, or say why it could not.
#[derive(Clone)]
pub struct Opener(Arc<Open>);

/// Showing `thread`, with the activation token the call carried.
type Open = dyn Fn(ThreadId, Option<&str>) -> Result<(), String> + Send + Sync;

impl std::fmt::Debug for Opener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Opener(..)")
    }
}

impl Opener {
    /// An opener that does `open`.
    pub fn new(
        open: impl Fn(ThreadId, Option<&str>) -> Result<(), String> + Send + Sync + 'static,
    ) -> Opener {
        Opener(Arc::new(open))
    }

    /// The window's: hand the conversation to the window that is running, as `mailo open` does,
    /// with the token so it may take the keyboard, or start one on it when none is.
    pub fn window() -> Opener {
        Opener::new(|thread, token| {
            if crate::ui::handoff::deliver(thread, token) {
                return Ok(());
            }
            let me = std::env::current_exe().map_err(|why| why.to_string())?;
            let mut started = std::process::Command::new(me);
            if let Some(token) = token {
                started.env("XDG_ACTIVATION_TOKEN", token);
            }
            let mut child = started
                .args(["open", &thread.to_string()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|why| format!("could not start a window: {why}"))?;
            // Waited for off this thread, so the window is not left a zombie when it closes.
            std::thread::spawn(move || child.wait());
            Ok(())
        })
    }
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Provider(..)")
    }
}

/// An outcome that says what it did, perhaps with the token that takes it back.
fn outcome(said: Option<String>, undo: Option<Token>, value: Option<Labelled<Output>>) -> Outcome {
    Outcome {
        value,
        said,
        show: Preview::None,
        undo: undo.map_or(Undoable::No, |token| Undoable::Yes(token.to_string())),
        follow: super::wire::Follow::Nothing,
    }
}

impl Provider {
    /// A provider over `store`, sending with `secrets`, opening conversations with `opener`.
    pub fn new(
        store: Arc<SqliteStore>,
        secrets: Arc<dyn SigningStore>,
        opener: Opener,
    ) -> Provider {
        Provider {
            store,
            secrets,
            stack: Mutex::new(UndoStack::default()),
            opener,
        }
    }

    /// The actions mailo answers, by the names the manifest gives them.
    pub fn actions() -> Vec<&'static str> {
        act::ALL.iter().map(|(name, _)| *name).collect()
    }

    /// `Perform`: do what the invocation names.
    pub fn perform(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        match Act::named(&invocation.action).ok_or(AppRefusal::Unsupported)? {
            Act::Search => self.search_action(invocation),
            Act::Read => self.read(invocation),
            Act::Open => self.open(invocation),
            Act::Contacts => self.contacts(invocation),
            Act::Thread(kind) => self.on_threads(kind, invocation),
            Act::CreateDraft => self.create_draft(invocation),
            Act::Send => self.send(invocation),
            Act::Forward => self.forward(invocation),
        }
    }

    /// `DryRun`: what the action would do, for the person to read first. Only what sends mail has
    /// anything to show.
    pub fn dry_run(&self, invocation: &Invocation) -> Result<Preview, AppRefusal> {
        match Act::named(&invocation.action).ok_or(AppRefusal::Unsupported)? {
            Act::Send => self.preview_send(invocation),
            Act::Forward => self.preview_forward(invocation),
            _ => Ok(Preview::None),
        }
    }

    /// `Undo`: take back what a token names. A token that is not mailo's, or was used, or came
    /// from a mailo that has stopped, is `Gone`.
    pub fn undo(&self, token: &str) -> Result<(), UndoFault> {
        match Token::parse(token).ok_or(UndoFault::Gone)? {
            Token::Stack(handle) => self.take_back_stack(handle),
            Token::Discard(draft) => self.discard(draft),
            Token::Unsend(drafts) => self.unsend(&drafts),
        }
    }

    /// `Search`: conversations a text finds.
    pub fn search(&self, text: &str) -> Vec<Hit> {
        self.search_hits(text)
    }

    /// `Preview`: a conversation, at a glance.
    pub fn preview(&self, entity: &EntityId) -> Preview {
        self.preview_of(entity)
    }

    /// `Suggest`: options for a parameter. Mailo has none to offer: a label or an address is
    /// typed.
    pub fn suggest(&self, _ask: &SuggestAsk) -> Vec<EntityRef> {
        Vec::new()
    }

    /// `Context`: what the person is looking at. A process with no window is looking at nothing,
    /// and says so as a private window, which the router reports as the app alone.
    pub fn context(&self) -> Context {
        Context {
            app: APP.to_owned(),
            window: Labelled {
                value: String::new(),
                label: Label::own(APP),
            },
            here: super::wire::Here::Nowhere,
            selection: super::wire::Selection::Nothing,
            visible: Visible {
                kind: None,
                items: Vec::new(),
                total: 0,
            },
            text_target: super::wire::TextTarget::None,
            privacy: Privacy::Private,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
