//! The account sheet: one account's servers and sign-in, and Remove Account, opened from its row
//! in the Connection Doctor.
//!
//! The settings are read, not edited: changing them is the Add account sheet's, which the
//! Doctor's Settings… already opens. Remove asks first, in the same sheet, naming how much mail
//! goes and whether the server still has it. Only the confirming button removes; Escape and
//! Cancel go back to the settings. A removal runs off the thread that draws (the keyring
//! blocks), and when it is done the account leaves every Space, the offline setting and the
//! tile pressed, the revision moves so the links and lists follow, and the Doctor comes back
//! without it. What the sheet says is [`words`]'s.

mod words;

use crate::ui::appearance::WindowDirs;
use crate::ui::common::in_card;
use crate::ui::data::account_rows;
use crate::ui::frame::{keep, scope_ids};
use crate::ui::press::{SheetClose, available, on_primary};
use crate::ui::space::{Forgot, Spaces, forget_account};
use crate::ui::view::{AccountSheet, AccountStep, Shell};
use dioxus::prelude::*;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::controls::button_model::{Answers, ButtonRole};
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::AccountId;
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// What the sheet is called.
const TITLE: &str = "Account Settings";

/// What the button that asks says.
const ASK: &str = "Remove Account\u{2026}";

/// The keyring a removal forgets the sign-in from, handed in as a context so tests reach their
/// own and never the user's.
#[derive(Clone)]
pub(in crate::ui) struct Seams {
    pub secrets: Arc<dyn Secrets>,
}

impl Seams {
    /// The system keyring. In this crate's tests, an empty keyring of its own.
    fn real() -> Seams {
        if cfg!(test) {
            return Seams {
                secrets: Arc::new(mail_runtime::MapSecrets::default()),
            };
        }
        Seams {
            secrets: Arc::new(mail_runtime::KeyringSecrets),
        }
    }
}

fn seams() -> Seams {
    try_consume_context::<Seams>().unwrap_or_else(Seams::real)
}

/// Open the sheet on `account`.
pub(in crate::ui) fn open(mut shell: Signal<Shell>, account: AccountId) {
    shell.write().account_sheet = Some(AccountSheet {
        account,
        step: AccountStep::Showing,
    });
}

/// Close the sheet. The Doctor, which stepped aside for it, comes back.
fn close(mut shell: Signal<Shell>) {
    shell.write().account_sheet = None;
    crate::ui::host::Host::focus_app();
}

/// Move the open sheet to `step`.
fn to(mut shell: Signal<Shell>, step: AccountStep) {
    if let Some(sheet) = shell.write().account_sheet.as_mut() {
        sheet.step = step;
    }
}

/// Escape: from the question back to the settings, from the settings closed. Nothing while a
/// removal is under way.
pub(in crate::ui) fn escape(shell: Signal<Shell>) {
    let step = shell
        .peek()
        .account_sheet
        .as_ref()
        .map(|sheet| sheet.step.clone());
    match step {
        Some(AccountStep::Asking) => to(shell, AccountStep::Showing),
        Some(AccountStep::Removing) | None => {}
        Some(AccountStep::Showing | AccountStep::Refused(_)) => close(shell),
    }
}

/// The confirming button: remove the account, then tidy what the window kept of it.
fn confirm(shell: Signal<Shell>, spaces: Signal<Spaces>, mut revision: Signal<u64>) {
    let Some(AccountSheet {
        account,
        step: AccountStep::Asking,
    }) = shell.peek().account_sheet.clone()
    else {
        return;
    };
    to(shell, AccountStep::Removing);
    let store = consume_context::<Arc<SqliteStore>>();
    let seams = seams();
    let dirs = try_consume_context::<WindowDirs>();
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            mail_core::account::remove(&store, seams.secrets.as_ref(), account)
        })
        .await;
        match done {
            Ok(Ok(_)) => {
                forgotten(shell, spaces, dirs.as_ref(), account);
                revision += 1;
                close(shell);
            }
            Ok(Err(error)) => to(shell, AccountStep::Refused(words::refused(&error))),
            Err(error) => to(
                shell,
                AccountStep::Refused(format!("It stopped before it finished: {error}")),
            ),
        }
    });
}

/// What the window kept of a removed account goes: from the Spaces, which are written, from
/// the tile pressed, and from the offline setting.
fn forgotten(
    mut shell: Signal<Shell>,
    mut spaces: Signal<Spaces>,
    dirs: Option<&WindowDirs>,
    account: AccountId,
) {
    let forgot = forget_account(&mut spaces.write(), account);
    if forgot == Forgot::Changed {
        keep(&spaces.read());
    }
    let scope = scope_ids(&spaces.read().current_space());
    let mut write = shell.write();
    write.scope = scope;
    if write.account == Some(account) {
        write.account = None;
    }
    drop(write);
    if let Some(dirs) = dirs {
        let _ = mail_core::offline::save(&dirs.config, account, mail_core::offline::Keep::Bodies);
    }
}

/// The sheet. Mounted while `shell.account_sheet` is `Some`.
#[component]
pub(in crate::ui) fn AccountSettingsSheet(
    shell: Signal<Shell>,
    spaces: Signal<Spaces>,
    revision: Signal<u64>,
) -> Element {
    let Some(sheet) = shell.read().account_sheet.clone() else {
        return rsx! {};
    };
    let store = consume_context::<Arc<SqliteStore>>();
    let row = account_rows(&store)
        .into_iter()
        .find(|row| row.id == sheet.account);
    let Some(row) = row else {
        // Removed from elsewhere while the sheet was open.
        return rsx! {
            Sheet {
                label: TITLE,
                attach: Attach::Window,
                common: in_card(),
                onclose: move |()| close(shell),
                div { class: "acct-sheet",
                    Label { text: words::refused(&mail_core::account::RemoveError::Unknown) }
                    div { class: "acct-foot",
                        SheetClose { label: "Done", on_close: move |()| close(shell) }
                    }
                }
            }
        };
    };
    let name = row.shown();
    let body = match &sheet.step {
        AccountStep::Asking | AccountStep::Removing => {
            let held = store.offline(row.id).map_or(0, |offline| {
                usize::try_from(offline.messages).unwrap_or(usize::MAX)
            });
            let asked = words::asking(&row.address, held, &row.plan.incoming);
            let removing = sheet.step == AccountStep::Removing;
            rsx! {
                div { class: "acct-done",
                    Label { text: asked.title.clone(), style: LabelStyle::Headline }
                    Label { text: asked.body }
                }
                div { class: "acct-foot",
                    // Escape is the sheet's own (`escape`), which brings the settings back; were
                    // Cancel to answer it too, one press would go back and then close.
                    Button {
                        label: "Cancel",
                        answers: Answers::Nothing,
                        availability: available(!removing),
                        onclick: on_primary(move || to(shell, AccountStep::Showing)),
                    }
                    Button {
                        label: asked.confirm,
                        role: ButtonRole::Destructive,
                        common: Common { aria_label: Some(asked.confirm.to_owned()), ..Common::default() },
                        availability: if removing { Availability::Busy } else { Availability::Enabled },
                        onclick: on_primary(move || confirm(shell, spaces, revision)),
                    }
                }
            }
        }
        AccountStep::Showing | AccountStep::Refused(_) => {
            let refused = match &sheet.step {
                AccountStep::Refused(why) => Some(why.clone()),
                _ => None,
            };
            rsx! {
                FieldGroup { title: "Servers",
                    for line in words::settings(&row.plan) {
                        FieldRow { key: "{line.label}", label: line.label,
                            Label { text: line.value }
                        }
                    }
                }
                if let Some(why) = refused {
                    Label { text: why, severity: Some(Severity::Warn) }
                }
                div { class: "acct-foot",
                    Button {
                        label: ASK,
                        role: ButtonRole::Destructive,
                        common: Common { aria_label: Some(format!("Remove {name}")), ..Common::default() },
                        onclick: on_primary(move || to(shell, AccountStep::Asking)),
                    }
                    SheetClose { label: "Done", on_close: move |()| close(shell) }
                }
            }
        }
    };
    rsx! {
        Sheet {
            label: TITLE,
            attach: Attach::Window,
            common: in_card(),
            onclose: move |()| escape(shell),
            div { class: "acct-sheet",
                Label { text: name.clone(), style: LabelStyle::Title }
                {body}
            }
        }
    }
}
