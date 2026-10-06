//! Accounts in this Space: one switch per account this computer has, on when the Space shows its
//! mail.
//!
//! The Space's, unlike the rows under it: a switch changes the draft like the look does, so the
//! list follows at once, Save keeps it and Escape puts it back.

use super::change;
use crate::ui::data::account_rows;
use crate::ui::space::edit::Draft;
use crate::ui::space::{Member, Spaces, with_member};
use dioxus::prelude::*;
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::prelude::*;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

#[component]
pub(super) fn Members(editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) -> Element {
    let accounts: Vec<(AccountId, String)> = use_hook(|| {
        try_consume_context::<Arc<SqliteStore>>()
            .map(|store| {
                account_rows(&store)
                    .into_iter()
                    .map(|row| (row.id.clone(), row.shown()))
                    .collect()
            })
            .unwrap_or_default()
    });
    let Some(scope) = editing
        .read()
        .as_ref()
        .map(|draft| draft.space.scope.clone())
    else {
        return rsx! {};
    };
    if accounts.is_empty() {
        return rsx! {};
    }
    let all: Vec<AccountId> = accounts.iter().map(|(id, _)| id.clone()).collect();
    rsx! {
        FieldGroup { title: "Accounts in this Space",
            for (id, name) in accounts {
                FieldRow { key: "{id}", label: name.clone(),
                    Toggle {
                        label: format!("Show {name} in this Space"),
                        value: if scope.shows(id.clone()) { Check::On } else { Check::Off },
                        onchange: {
                            let all = all.clone();
                            let member = if scope.shows(id.clone()) { Member::Out } else { Member::In };
                            move |_| {
                                change(editing, spaces, |draft| {
                                    draft.space.scope = with_member(&draft.space.scope, id.clone(), member, &all);
                                });
                            }
                        },
                    }
                }
            }
        }
    }
}
