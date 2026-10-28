//! Redirect…: the conversation's newest message, unchanged, to whoever is typed here.
//!
//! A popover with one field, as Choose Time… is: the addresses as the To row takes them, and
//! Return sends. Nothing is composed, so there is no composer to open; what goes is the message
//! as it came, beneath the `Resent-` block that says it was passed on, and when, and by whom
//! (`mail_core::compose::redirect`). A typo is the field's own refusal and leaves the popover up;
//! anything the core refuses (a body not here, an account that sends through Graph) is said as a
//! toast, as every other outcome of a send is. A redirect that is queued waits out the same grace
//! as a send, and its toast's Undo withdraws it.

use super::super::menu::{MenuKey, anchor_for, menu_key};
use super::super::motion::{Follow, Restore, tell};
use super::super::ops::newest;
use crate::ui::copy;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::fields::text_field_model::Invalid;
use ds::components::overlays::popover::Arrow;
use ds::host::measure::MountedRef;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use mail_client::Client;
use mail_domain::*;

/// What pressing Return with `typed` in the field comes to: the addresses, or why there are none.
pub(super) fn recipients(typed: &str) -> Result<Vec<Address>, String> {
    let to = mail_client::compose::addresses::parse_addresses(typed).map_err(String::from)?;
    if to.is_empty() {
        return Err(copy::fields::REDIRECT_HELP.to_owned());
    }
    Ok(to)
}

/// Redirect `id`'s newest message to `to`, and say how it went: with an Undo while it waits.
fn redirect(client: &Client, id: ThreadId, to: &[Address]) -> (String, Follow) {
    let grace = crate::ui::prefs::current().compose.undo_send.grace();
    let went = newest(client, id).and_then(|message| {
        client
            .compose()
            .redirect(message, to, grace, chrono::Utc::now())
            .map_err(String::from)
    });
    match went {
        Ok(redirected) => (
            copy::toasts::redirected(&redirected.to),
            Follow::Restore(Restore::Redirect(redirected)),
        ),
        Err(why) => (copy::toasts::not_redirected(&why), Follow::Nothing),
    }
}

/// The popover, anchored where the menu that asked for it stood.
#[component]
pub(in crate::ui) fn RedirectMenu(
    id: ThreadId,
    shell: Signal<Shell>,
    anchor: Option<MountedRef>,
    #[props(default)] placed: Option<Rect>,
) -> Element {
    let mut typed = use_signal(String::new);
    // The refusal of the last Return, shown until the field changes.
    let mut refused = use_signal(|| None::<String>);
    let mut done = move || shell.write().further = None;
    let validity = match refused() {
        Some(why) => Validity::Invalid(Invalid {
            message: why.into(),
            stamp: EventStamp(u32::try_from(typed().len()).unwrap_or(0)),
        }),
        None => Validity::Valid,
    };
    let help = refused()
        .is_none()
        .then(|| TextLine::from(copy::fields::REDIRECT_HELP.to_owned()));
    rsx! {
        Popover {
            anchor: anchor_for(anchor, placed),
            placement: Placement::new(Side::Bottom, Align::Start),
            gap: Px(4.0),
            arrow: Arrow::None,
            onclose: move |()| done(),
            div {
                class: "pick-time redirect",
                onkeydown: move |event: KeyboardEvent| {
                    match menu_key(&event.key().to_string()) {
                        Some(MenuKey::Enter) => {
                            event.prevent_default();
                            match recipients(&typed()) {
                                Ok(to) => {
                                    let client = crate::ui::live::client();
                                    let (said, follow) = redirect(&client, id, &to);
                                    done();
                                    tell(said, follow);
                                }
                                Err(why) => refused.set(Some(why)),
                            }
                        }
                        Some(MenuKey::Escape) => {
                            event.stop_propagation();
                            done();
                        }
                        _ => {}
                    }
                },
                TextField {
                    label: copy::fields::REDIRECT_TO.to_owned(),
                    placeholder: copy::fields::SOMEONE.to_owned(),
                    value: typed(),
                    validity,
                    help,
                    focus: FieldFocus::OnMount,
                    oninput: move |value: String| {
                        typed.set(value);
                        refused.set(None);
                    },
                    common: Common { extra_class: ExtraClass::parse("pick-field").ok(), ..Common::default() },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_takes_addresses_as_the_to_row_does_and_refuses_none() {
        let to = recipients("Bea <bea@example.test>, cy@example.test").unwrap();
        let emails: Vec<&str> = to.iter().map(|a| a.email.as_str()).collect();
        assert_eq!(emails, ["bea@example.test", "cy@example.test"]);
        assert!(recipients("  ").is_err());
        assert!(recipients("not an address").is_err());
    }
}
