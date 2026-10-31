//! A message's one-time code in the reader: one quiet line under its header, the code and Copy
//! Code, for the verification and sign-in mail a person opens only to copy a number out of.
//!
//! Which code a message carries is [`mail_logic::otp`]'s. Finding it may read the stored
//! blob (most of this mail is HTML alone), so it runs off the thread that draws and is kept per
//! message and body in the app's [`Looks`](mail_core::message::Looks), as an invitation is.
//! Copy Code goes through `ui::sensitive_copy`, not the window's host: the code is kept out of
//! clipboard history and cloud sync, which the host's plain copy does not do.

use crate::ui::copy::{controls, status, toasts};
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::Bezel;
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::{BlobId, MessageId};

/// The line for one message, once its code is known; nothing for a message with none.
///
/// Keyed by the reader on the message and its body, so a body arriving looks again and one
/// message's code is never drawn under another.
#[component]
pub(super) fn CodeLine(message: MessageId, body: Option<BlobId>) -> Element {
    let looks = super::use_looks();
    let mut known = use_signal({
        let looks = looks.clone();
        move || looks.code_cached(message, body)
    });
    let _look = use_resource(move || {
        let client = crate::ui::live::client();
        let looks = looks.clone();
        async move {
            if known.peek().is_some() {
                return;
            }
            let found =
                tokio::task::spawn_blocking(move || client.looking(&looks).code(message, body))
                    .await
                    // A lookup that panicked found nothing to offer.
                    .unwrap_or_default();
            known.set(Some(found));
        }
    });
    let Some(Some(code)) = known() else {
        return rsx! {};
    };
    let label = controls::copy_code(&code);
    let shown = status::one_time_code(&code);
    let copied = code.clone();
    rsx! {
        p { class: "otp-code", "data-code": "{code}",
            Label { text: shown, role: LabelRole::Secondary, style: LabelStyle::Footnote }
            Button {
                bezel: Bezel::Inline,
                label: controls::COPY_CODE,
                common: Common { aria_label: Some(label), ..Common::default() },
                onclick: on_primary(move || {
                    crate::ui::sensitive_copy::copy(&copied);
                    crate::ui::motion::tell(toasts::CODE_COPIED.to_owned(), crate::ui::motion::Follow::Nothing);
                }),
            }
        }
    }
}
