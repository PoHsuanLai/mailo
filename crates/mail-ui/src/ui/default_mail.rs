//! Making mailo the application that opens email links: the question asked once at launch, and
//! Settings › General's row, which always says which application it is now and offers it again.
//!
//! Both read the root context [`MailHandlers`] (`mail_os::mail_handler`): the system's in the
//! launched window, a fake in the tests. Without one (a test that hands none, a platform where
//! mailo cannot be chosen from inside, a `mailo` that is not an installed app) neither is drawn.
//!
//! The launch question is asked once, whatever the answer: Not Now is an answer, and a window that
//! asked again at every launch would be nagging. That it was asked is kept in the state directory
//! ([`FILE_NAME`]), beside Today's file. It waits for an account, so a first run's Add Account is
//! not interrupted by it.

use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_os::mail_handler::{Handler, Made, MailHandlers};
use mail_ui_kit::appearance::WindowDirs;
use mail_ui_kit::press::on_primary;
use mail_ui_kit::view::Shell;
use mail_ui_model::copy;
use std::path::Path;

/// Settings' Make Default button.
pub const MAKE_DEFAULT_ID: &str = "default-mail-make";

/// The launch question's two buttons.
pub const ASK_MAKE_ID: &str = "default-mail-ask-make";
pub const ASK_LATER_ID: &str = "default-mail-ask-later";

/// Where the launch question's answer is kept, in the state directory.
pub const FILE_NAME: &str = "default-mail.json";

/// Whether the launch question has been asked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Asked {
    /// Not yet: it is asked at the next launch that has an account.
    #[default]
    NotYet,
    /// Asked, and answered either way.
    Answered,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    asked: Asked,
}

/// Whether the question was asked, from `dir`. Not yet when there is no file or it cannot be read.
pub fn load(dir: &Path) -> Asked {
    mail_client::config::read_json::<Stored>(dir, FILE_NAME).asked
}

/// Keep `asked` in `dir`.
pub fn save(dir: &Path, asked: Asked) -> Result<(), String> {
    mail_client::config::write_json(dir, FILE_NAME, &Stored { asked }).map_err(String::from)
}

/// The words for `handler` in Settings' row.
pub fn name_of(handler: &Handler) -> String {
    match handler {
        Handler::Mailo => copy::fields::THIS_APP.to_owned(),
        Handler::Other(name) => name.clone(),
        Handler::Nobody => copy::fields::NO_MAIL_APP.to_owned(),
    }
}

/// Ask the system to make mailo the handler, and hear the end of it on the window's thread.
async fn make_mailo(handlers: &MailHandlers) -> Made {
    let (tell, heard) = tokio::sync::oneshot::channel();
    handlers.0.make_mailo(Box::new(move |made| {
        let _ = tell.send(made);
    }));
    heard.await.unwrap_or(Made::Kept)
}

/// Where the launch question stands in this window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Question {
    /// Not decided yet: the window has no account.
    Waiting,
    /// On screen.
    Open,
    /// Answered, or never to be asked here.
    Done,
}

/// The launch question: an alert, once, while another application opens email links and there is
/// an account to write from.
#[component]
pub fn DefaultMailQuestion(shell: Signal<Shell>) -> Element {
    let handlers = try_consume_context::<MailHandlers>();
    let dirs = try_consume_context::<WindowDirs>();
    let mut question = use_signal(|| Question::Waiting);
    // Decided once, when the window first has an account: the system is asked which application
    // opens the links only then, and never again in this window.
    use_effect({
        let handlers = handlers.clone();
        let dirs = dirs.clone();
        move || {
            let has_account = !shell.read().accounts.is_empty();
            if !has_account || *question.peek() != Question::Waiting {
                return;
            }
            let ask = match (handlers.as_ref(), dirs.as_ref()) {
                (Some(handlers), Some(dirs)) => {
                    load(&dirs.state) == Asked::NotYet && handlers.0.current() != Handler::Mailo
                }
                _ => false,
            };
            question.set(if ask { Question::Open } else { Question::Done });
        }
    });
    let (Some(handlers), Some(dirs)) = (handlers, dirs) else {
        return rsx! {};
    };
    if question() != Question::Open {
        return rsx! {};
    }
    let answered = {
        let state = dirs.state.clone();
        move || {
            let _ = save(&state, Asked::Answered);
            question.set(Question::Done);
        }
    };
    let mut on_make = answered.clone();
    let mut on_later = answered;
    rsx! {
        Alert {
            title: copy::confirm::MAKE_DEFAULT_MAIL.to_owned(),
            message: Some(TextLine::from(copy::confirm::MAKE_DEFAULT_MAIL_BODY)),
            style: AlertStyle::Informational,
            buttons: vec![
                AlertButton::new(copy::controls::MAKE_DEFAULT, AlertRole::Normal, EventHandler::new(move |()| {
                    on_make();
                    let handlers = handlers.clone();
                    spawn(async move {
                        make_mailo(&handlers).await;
                    });
                })).with_common(Common { id: Some(ASK_MAKE_ID.to_owned()), ..Common::default() }),
                AlertButton::new(copy::controls::NOT_NOW, AlertRole::Cancel, EventHandler::new(move |()| {
                    on_later();
                })).with_common(Common { id: Some(ASK_LATER_ID.to_owned()), ..Common::default() }),
            ],
        }
    }
}

/// Settings › General's row: which application opens email links, and Make Default while it is
/// not mailo. Read again after each request, whatever the person chose in the system's dialog.
#[component]
pub fn DefaultMailRow() -> Element {
    let Some(handlers) = try_consume_context::<MailHandlers>() else {
        return rsx! {};
    };
    let mut handler = use_signal({
        let handlers = handlers.clone();
        move || handlers.0.current()
    });
    let mailo = handler() == Handler::Mailo;
    rsx! {
        FieldRow { label: copy::fields::DEFAULT_MAIL_APP,
            Label { text: name_of(&handler()) }
            if !mailo {
                Button {
                    label: copy::controls::MAKE_DEFAULT,
                    size: ControlSize::Small,
                    common: Common { id: Some(MAKE_DEFAULT_ID.to_owned()), ..Common::default() },
                    onclick: on_primary(move || {
                        let handlers = handlers.clone();
                        spawn(async move {
                            make_mailo(&handlers).await;
                            handler.set(handlers.0.current());
                        });
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "default_mail_tests.rs"]
mod tests;
