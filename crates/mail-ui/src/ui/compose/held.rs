//! A message held to leave at a time, opened from its row: what it says, and when it leaves.
//!
//! It is read here and not written: the outbox holds it as it was sealed, so an edit could not
//! reach what leaves. The bar over it names the time and offers Cancel Sending, which makes it a
//! draft again; the page then opens it in the composer, where it can be changed and sent anew.

use super::List;
use super::props::{Line, list_label};
use crate::ui::menus::when_words;
use crate::ui::press::on_primary;
use crate::ui::view::Shell;
use chrono::{DateTime, Local, Utc};
use dioxus::prelude::*;
use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::compose::addresses::join_addresses;
use mail_domain::{Address, Draft};
use mail_ui_model::copy;

/// `draft`, held to leave `at`.
#[component]
pub(super) fn HeldPage(
    draft: Draft,
    at: DateTime<Utc>,
    shell: Signal<Shell>,
    /// Cancel Sending went through: the message is a draft again, to be written.
    on_cancelled: EventHandler<()>,
) -> Element {
    let id = draft.id;
    let subject = mail_domain::subject_or_none(&draft.subject).into_owned();
    let leaves = copy::status::leaves(&when_words(
        at,
        Utc::now(),
        &Local,
        crate::ui::clock::hours(),
    ));
    let lines = [
        (List::To, &draft.to),
        (List::Cc, &draft.cc),
        (List::Bcc, &draft.bcc),
    ]
    .into_iter()
    .filter(|(_, who)| !who.is_empty())
    .map(|(list, who): (List, &Vec<Address>)| (list, join_addresses(who)))
    .collect::<Vec<_>>();
    rsx! {
        div { class: "cpage held", "data-held": "{subject}",
            div { class: "c-scroll",
                InlineBanner {
                    severity: Severity::Info,
                    icon: Some(Icon::Clock),
                    text: leaves,
                    actions: rsx! {
                        Button {
                            label: copy::menus::CANCEL_SENDING,
                            common: Common {
                                aria_label: Some(copy::menus::CANCEL_SENDING.to_owned()),
                                ..Common::default()
                            },
                            // A refusal is said in the toast; the message stays as it was.
                            onclick: on_primary(move || {
                                if super::cancel_scheduled(shell, id).is_ok() {
                                    on_cancelled.call(());
                                }
                            }),
                        }
                    },
                }
                div { class: "c-head",
                    h1 { class: "c-title", "{subject}" }
                }
                div { class: "c-props",
                    for (list, who) in lines {
                        Line { key: "{list_label(list)}", label: list_label(list), row: list_label(list),
                            span { "{who}" }
                        }
                    }
                }
                div { class: "c-main",
                    div { class: "held-text", "{draft.text}" }
                }
            }
        }
    }
}
