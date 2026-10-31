//! The parts of an account's page the person changes: what the account is called, whether it
//! syncs, and which folder plays each role.
//!
//! The description and the switch are kept the moment they change, in the client's
//! `account_prefs`, and the revision moves, so the sidebar's tile and every list that names the
//! account follow at once. A paused account is left out of every sync (`mail_core::sync::paused`)
//! and its tile's mark says so.
//!
//! A folder picked for a role is laid over what the server says (the client's `with_picks`), so
//! the next refresh of the server's folders keeps it; "Automatic" gives the role back.

use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::menus::item::item::MenuItem;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::Syncing;
use mail_domain::MailboxRole;
use mail_ui_kit::common::{Problem, classed};
use mail_ui_kit::data::AccountRow;
use mail_ui_model::copy;
use mail_ui_model::copy::fields::account as field;
use mail_ui_model::copy::status::account as said;

/// Write one preference with `write`, off the thread that draws (the store's writer may be
/// held by a sync): what names the account reads it again when the client sees the store move,
/// and a write that fails is said in `failed`.
fn keep(
    write: impl FnOnce(&mail_client::Stored<'_>) -> Result<(), mail_client::prelude::StoreError>
    + Send
    + 'static,
    mut failed: Signal<Option<String>>,
) {
    let Some(client) = mail_ui_kit::live::try_client() else {
        return;
    };
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || write(&client.stored())).await;
        match done {
            Ok(Ok(())) => failed.set(None),
            Ok(Err(why)) => failed.set(Some(why.to_string())),
            Err(why) => failed.set(Some(why.to_string())),
        }
    });
}

/// The account's description and its Sync switch.
#[component]
pub(super) fn About(row: AccountRow) -> Element {
    let failed = use_signal(|| None::<String>);
    let mut typed = use_signal({
        let had = row.description.clone();
        // No description yet is an empty field, the address its placeholder.
        move || had.unwrap_or_default()
    });
    let syncing = row.syncing;
    let (keeps_offered, keeps) = keeping_labels(&row);
    let labelled = row.id.clone();
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
                            move |stored| stored.set_account_description(account, Some(&value)),
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
                        keep(move |stored| stored.set_account_syncing(account, to), failed);
                    },
                }
            }
        }
        if keeps_offered {
            FormSection {
                FieldRow {
                    label: field::LABELS,
                    help: Some(TextLine::from(said::KEEP_LABELS_HELP)),
                    Toggle {
                        label: field::KEEP_LABELS_ON_THE_SERVER,
                        value: if keeps { Check::On } else { Check::Off },
                        onchange: move |now: Check| {
                            let account = labelled.clone();
                            keep(
                                move |stored| stored.set_account_keeps_labels(account, now == Check::On),
                                failed,
                            );
                        },
                    }
                }
            }
        }
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| { let mut failed = failed; failed.set(None) } }
        }
    }
}

/// Whether labels can be kept on `row`'s server as keywords, and whether the person said to: an
/// IMAP account whose server is not Gmail, which has labels of its own.
fn keeping_labels(row: &AccountRow) -> (bool, bool) {
    let Some(client) = mail_ui_kit::live::try_client() else {
        return (false, false);
    };
    let offered = row.plan.incoming.caps().writes_mailboxes
        && client
            .sync()
            .caps_of(row.id.clone())
            .is_some_and(|caps| caps.labels != mail_domain::ServerLabels::Supported);
    let keeps = client
        .stored()
        .account_prefs(row.id.clone())
        .is_ok_and(|prefs| prefs.keep_labels);
    (offered, keeps)
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
pub(super) fn FolderRoles(row: AccountRow) -> Element {
    let failed = use_signal(|| None::<String>);
    let mut unread_seen = use_signal(|| false);
    let Some(client) = mail_ui_kit::live::try_client() else {
        return rsx! {};
    };
    if !row.plan.incoming.caps().lists_folders {
        return rsx! {};
    }
    // What cannot be read is said, once, and the group is not drawn: a menu drawn from nothing
    // would offer Automatic over a pick the person made.
    let read = client.stored().folders(row.id.clone()).and_then(|folders| {
        Ok((
            folders,
            client.stored().account_prefs(row.id.clone())?.folders,
        ))
    });
    let (folders, picks) = match read {
        Ok(read) => read,
        Err(why) if !unread_seen() => {
            return rsx! {
                Problem { title: copy::titles::NOT_READ, why: why.to_string(), onclose: move |()| unread_seen.set(true) }
            };
        }
        Err(_) => return rsx! {},
    };
    // Before the first listing there is nothing to pick from, and the group is not drawn.
    if folders.is_empty() {
        return rsx! {};
    }
    let observed = client
        .sync()
        .caps_of(row.id.clone())
        .map(|caps| caps.folders);
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
                                common: Common { aria_label: Some(copy::aria::folder_for(label)), ..Common::default() },
                                onpick: move |path: Option<String>| {
                                    let account = account.clone();
                                    keep(move |stored| stored.set_folder_role(account, role, path.as_deref()), failed);
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
