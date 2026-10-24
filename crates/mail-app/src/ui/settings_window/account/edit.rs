//! The parts of an account's page the person changes: what the account is called and whether
//! it syncs, its servers, and which folder plays each role.
//!
//! The description and the switch are kept the moment they change, in the store's
//! `account_prefs`, and the revision moves, so the sidebar's tile and every list that names the
//! account follow at once. A paused account is left out of every sync (`mail_core::sync::paused`)
//! and its tile's mark says so.
//!
//! The servers are kept only on Save, and only once the incoming server has taken a sign-in
//! with them ([`mail_core::account::replan`], the check Add Account makes). The check runs on a
//! thread of its own, since it waits on the network; a refusal is drawn under the row it is
//! about, in the server's words, and the account keeps the settings it had.
//!
//! A folder picked for a role is laid over what the server says (the store's `with_picks`), so
//! the next refresh of the server's folders keeps it; "Automatic" gives the role back.

use super::servers::{self, Receiving, Refusal, Row, Server, ServerForm};
use crate::ui::common::{Problem, classed};
use crate::ui::copy;
use crate::ui::copy::fields::account as field;
use crate::ui::copy::status::account as said;
use crate::ui::data::AccountRow;
use crate::ui::press::on_primary;
use chrono::Utc;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::fields::text_field_model::Invalid;
use ds::components::menus::item::item::MenuItem;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::{SqliteStore, Store, Syncing};
use mail_domain::{Incoming, MailboxRole, Tls};
use std::sync::Arc;

/// Write one preference with `write`, then move the revision so everything that names the
/// account reads it again; a store that refuses is said in `failed`.
fn keep(
    write: impl FnOnce(&SqliteStore) -> Result<(), mail_core::StoreError>,
    mut revision: Signal<u64>,
    mut failed: Signal<Option<String>>,
) {
    let Some(store) = try_consume_context::<Arc<SqliteStore>>() else {
        return;
    };
    match write(&store) {
        Ok(()) => {
            failed.set(None);
            revision += 1;
        }
        Err(why) => failed.set(Some(why.to_string())),
    }
}

/// The account's description and its Sync switch.
#[component]
pub(super) fn About(row: AccountRow, revision: Signal<u64>) -> Element {
    let failed = use_signal(|| None::<String>);
    let mut typed = use_signal({
        let had = row.description.clone();
        // No description yet is an empty field, the address its placeholder.
        move || had.unwrap_or_default()
    });
    let syncing = row.syncing;
    let id = row.id.clone();
    let described = id.clone();
    rsx! {
        FormSection {
            FieldRow { label: field::DESCRIPTION, help: Some(TextLine::from(said::DESCRIPTION_HELP)),
                TextField {
                    label: field::DESCRIPTION.to_owned(),
                    value: typed(),
                    placeholder: row.address.clone(),
                    common: Common { id: Some("acct-description".to_owned()), ..classed("acct-field") },
                    oninput: move |value: String| typed.set(value),
                    onchange: move |value: String| {
                        let account = described.clone();
                        keep(
                            move |store| store.set_account_description(account, Some(&value)),
                            revision,
                            failed,
                        );
                    },
                }
            }
            FieldRow {
                label: field::SYNC,
                help: (syncing == Syncing::Paused).then(|| TextLine::from(said::PAUSED_HELP)),
                Toggle {
                    label: field::SYNC_THIS_ACCOUNT,
                    value: if syncing == Syncing::On { Check::On } else { Check::Off },
                    onchange: move |now: Check| {
                        let account = id.clone();
                        let to = if now == Check::On { Syncing::On } else { Syncing::Paused };
                        keep(move |store| store.set_account_syncing(account, to), revision, failed);
                    },
                }
            }
        }
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| { let mut failed = failed; failed.set(None) } }
        }
    }
}

/// What a refusal at `row` draws under that row's field, and nothing under any other.
fn validity(refused: Option<&Refusal>, row: Row) -> Validity {
    match refused {
        Some(refusal) if refusal.row == row => Validity::Invalid(Invalid {
            message: refusal.why.clone().into(),
            stamp: EventStamp(u32::try_from(refusal.why.len()).unwrap_or(0)),
        }),
        _ => Validity::Valid,
    }
}

/// The security menu's choices, as the menu names them.
fn securities() -> Vec<MenuItem<Tls>> {
    vec![
        MenuItem::new(Tls::Implicit, said::TLS_CHOICE),
        MenuItem::new(Tls::StartTlsRequired, said::STARTTLS_CHOICE),
        MenuItem::new(Tls::Plaintext, said::NONE_CHOICE),
    ]
}

/// One server's three rows: its host, its port and its security. `incoming` says which of the
/// form's servers they edit, and which refusals are theirs.
#[component]
fn ServerRows(
    label: &'static str,
    server: Server,
    incoming: bool,
    form: Signal<ServerForm>,
    refused: Signal<Option<Refusal>>,
    availability: Availability,
) -> Element {
    let (host_row, port_row) = if incoming {
        (Row::Host, Row::Port)
    } else {
        (Row::OutgoingHost, Row::OutgoingPort)
    };
    let mut edit = move |change: &dyn Fn(&mut Server)| {
        let mut write = form.write();
        let target = if incoming {
            match &mut write.incoming {
                Receiving::Server(server) => Some(server),
                Receiving::Session(_) | Receiving::Fixed => None,
            }
        } else {
            write.outgoing.as_mut()
        };
        if let Some(target) = target {
            change(target);
        }
    };
    let which = if incoming { "in" } else { "out" };
    let refusal = refused.read().clone();
    rsx! {
        FieldRow { label,
            TextField {
                label: format!("{label} {}", copy::fields::account::INCOMING_SERVER.to_lowercase().split(' ').next_back().unwrap_or("server")),
                value: server.host.clone(),
                availability,
                validity: validity(refusal.as_ref(), host_row),
                common: Common { id: Some(format!("acct-{which}-host")), ..classed("acct-field") },
                oninput: move |value: String| edit(&|server| server.host = value.clone()),
            }
        }
        FieldRow { label: field::PORT,
            TextField {
                label: format!("{label} {}", field::PORT.to_lowercase()),
                value: server.port.clone(),
                availability,
                validity: validity(refusal.as_ref(), port_row),
                common: Common { id: Some(format!("acct-{which}-port")), ..classed("acct-port") },
                oninput: move |value: String| edit(&|server| server.port = value.clone()),
            }
        }
        FieldRow { label: field::SECURITY,
            PopUpButton::<Tls> {
                items: securities(),
                value: Some(server.tls),
                availability,
                common: Common { aria_label: Some(format!("{label} {}", field::SECURITY.to_lowercase())), ..Common::default() },
                onpick: move |tls: Tls| edit(&|server| server.tls = tls),
            }
        }
    }
}

/// The servers, editable where they were typed by hand, with Save under them; the rows a
/// provider decided are read-only, as they always were.
#[component]
pub(super) fn Servers(row: AccountRow, revision: Signal<u64>) -> Element {
    let had = servers::form_of(&row.plan);
    let mut form = use_signal(|| had.clone());
    let mut refused = use_signal(|| None::<Refusal>);
    let mut checking = use_signal(|| false);
    let current = form.read().clone();
    let fixed: Vec<super::words::Line> = super::words::settings(&row.plan)
        .into_iter()
        .filter(|line| !shown_editable(&current, line.label))
        .collect();
    let availability = if checking() {
        Availability::Busy
    } else {
        Availability::Enabled
    };
    let changed = servers::plan_from(&row.plan, &current).map_or(true, |plan| plan != row.plan);
    let save = {
        let row = row.clone();
        move || {
            let plan = match servers::plan_from(&row.plan, &form.peek()) {
                Ok(plan) => plan,
                Err(refusal) => {
                    refused.set(Some(refusal));
                    return;
                }
            };
            let Some(store) = try_consume_context::<Arc<SqliteStore>>() else {
                return;
            };
            let secrets = super::seams().secrets;
            let account = row.id.clone();
            refused.set(None);
            checking.set(true);
            // A page left mid-check drops this task; the thread finishes the check, and puts the
            // old plan back if it fails, on its own.
            spawn(async move {
                let done = crate::edge::on_thread("mailo-check-servers", move || async move {
                    mail_core::account::replan(&store, account, &plan, secrets, Utc::now())
                        .await
                        .map_err(|why| servers::refusal_of(&why))
                })
                .await;
                checking.set(false);
                match done {
                    Some(Ok(())) => revision += 1,
                    Some(Err(refusal)) => refused.set(Some(refusal)),
                    None => refused.set(Some(Refusal {
                        row: Row::Host,
                        why: copy::errors::CHECK_STOPPED.to_owned(),
                    })),
                }
            });
        }
    };
    let refusal = refused.read().clone();
    rsx! {
        FormSection { title: Some(copy::headers::SERVERS.to_owned()),
            match current.incoming.clone() {
                Receiving::Server(server) => rsx! {
                    ServerRows { label: field::INCOMING_SERVER, server, incoming: true, form, refused, availability }
                },
                Receiving::Session(session) => rsx! {
                    FieldRow { label: field::SESSION_URL,
                        TextField {
                            label: field::SESSION_URL.to_owned(),
                            value: session,
                            availability,
                            validity: validity(refusal.as_ref(), Row::Host),
                            common: Common { id: Some("acct-in-host".to_owned()), ..classed("acct-field") },
                            oninput: move |value: String| form.write().incoming = Receiving::Session(value),
                        }
                    }
                },
                Receiving::Fixed => rsx! {},
            }
            if let Some(server) = current.outgoing.clone() {
                ServerRows { label: field::OUTGOING_SERVER, server, incoming: false, form, refused, availability }
            }
            if let Some(username) = current.username.clone() {
                FieldRow { label: field::USER_NAME,
                    TextField {
                        label: field::USER_NAME.to_owned(),
                        value: username,
                        availability,
                        validity: validity(refusal.as_ref(), Row::Username),
                        common: Common { id: Some("acct-username".to_owned()), ..classed("acct-field") },
                        oninput: move |value: String| form.write().username = Some(value),
                    }
                }
            }
            for line in fixed {
                FieldRow { key: "{line.label}", label: line.label,
                    Label { text: line.value }
                }
            }
        }
        if servers::editable(&current) {
            div { class: "acct-save",
                Label { text: said::SAVE_HELP }
                Button {
                    label: copy::controls::SAVE,
                    common: Common { id: Some("acct-save-servers".to_owned()), ..Common::default() },
                    availability: if checking() { Availability::Busy } else if changed { Availability::Enabled } else { Availability::Disabled },
                    onclick: on_primary(save),
                }
            }
        }
    }
}

/// Whether the read-only line `label` is one the form already draws as fields.
fn shown_editable(form: &ServerForm, label: &str) -> bool {
    match label {
        field::RECEIVING => form.incoming != Receiving::Fixed,
        field::SENDING => form.outgoing.is_some(),
        field::SIGNS_IN => form.username.is_some(),
        _ => false,
    }
}

/// The roles a person may give a folder, in the order the page lists them, with their words.
const ROLES: [(MailboxRole, &str); 5] = [
    (MailboxRole::Drafts, field::DRAFTS),
    (MailboxRole::Sent, field::SENT),
    (MailboxRole::Spam, field::JUNK),
    (MailboxRole::Trash, field::TRASH),
    (MailboxRole::Archive, field::ARCHIVE),
];

/// Which folder plays each role, for an account whose server has folders and has listed them.
#[component]
pub(super) fn FolderRoles(row: AccountRow, revision: Signal<u64>) -> Element {
    let failed = use_signal(|| None::<String>);
    let Some(store) = try_consume_context::<Arc<SqliteStore>>() else {
        return rsx! {};
    };
    if !matches!(
        row.plan.incoming,
        Incoming::Imap { .. } | Incoming::Jmap { .. } | Incoming::Graph
    ) {
        return rsx! {};
    }
    // Before the first listing there is nothing to pick from, and the group is not drawn.
    let folders = store.folders(row.id.clone()).unwrap_or_default();
    if folders.is_empty() {
        return rsx! {};
    }
    // Unreadable preferences pick nothing, which is how the account behaves then too.
    let picks = store
        .account_prefs(row.id.clone())
        .unwrap_or_default()
        .folders;
    let observed = mail_core::sync::caps_of(&store, row.id.clone()).map(|caps| caps.folders);
    let paths: Vec<String> = folders
        .into_iter()
        .filter(|folder| !folder.path.eq_ignore_ascii_case("INBOX"))
        .map(|folder| folder.path)
        .collect();
    rsx! {
        FormSection { title: Some(copy::headers::FOLDERS.to_owned()),
            for (role, label) in ROLES {
                {
                    let picked = picks.iter().find(|(had, _)| *had == role).map(|(_, path)| path.clone());
                    let automatic = match &picked {
                        Some(_) => said::automatic(None),
                        None => said::automatic(observed.as_ref().and_then(|roles| roles.path(role))),
                    };
                    let mut items = vec![MenuItem::new(None, automatic)];
                    items.extend(paths.iter().map(|path| MenuItem::new(Some(path.clone()), path.clone())));
                    let account = row.id.clone();
                    rsx! {
                        FieldRow { key: "{label}", label,
                            PopUpButton::<Option<String>> {
                                items,
                                value: Some(picked),
                                common: Common { aria_label: Some(format!("{label} folder")), ..Common::default() },
                                onpick: move |path: Option<String>| {
                                    let account = account.clone();
                                    keep(move |store| store.set_folder_role(account, role, path.as_deref()), revision, failed);
                                },
                            }
                        }
                    }
                }
            }
        }
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| { let mut failed = failed; failed.set(None) } }
        }
    }
}
