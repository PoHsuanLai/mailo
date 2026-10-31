//! VIPs in the window: the mark beside a VIP's name, in a row and in the reader, and marking or
//! unmarking a sender from the reader's name menu, a row's menu or Settings.
//!
//! Who is a VIP is the client's (`mail_core::vip`); the window holds a copy in [`Shell::vips`],
//! read again on every revision as the labels are, so a mark made anywhere moves the rows, the
//! reader and the VIPs place together.

use crate::ui::motion::tell_through;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::style::icon::render::Glyph;
use mail_client::vip::Mark;
use mail_ui_model::copy;

/// Where a mark made in the reader lands: the window's shell.
/// The reader provides it, so the sender's name below it can mark without being handed both.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Marking {
    pub shell: Signal<Shell>,
}

/// Whether `address` may be marked a VIP: anyone's but the person's own, whose mail is not mail
/// to be heard of.
pub(in crate::ui) fn markable(shell: &Shell, address: &str) -> bool {
    !shell
        .accounts
        .iter()
        .any(|(own, _)| own.trim().eq_ignore_ascii_case(address.trim()))
}

/// Mark `address` a VIP (`name` the name the mail gave) or take the mark off, and say only a
/// failure. The write runs on a blocking thread; the shell's copy is read again when it has
/// landed, and the store moving makes the VIPs place and every other window follow.
pub(in crate::ui) fn set(marking: Marking, address: &str, name: Option<&str>, mark: Mark) {
    let Marking { mut shell } = marking;
    let client = crate::ui::live::client();
    let motion = crate::ui::motion::motion();
    let (address, name) = (address.to_owned(), name.map(str::to_owned));
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            client
                .query()
                .set_vip(&address, name.as_deref(), mark)
                .map(|()| client.query().vips())
                .map_err(crate::ui::remedy::told)
        })
        .await
        .unwrap_or_else(|why| Err(why.to_string()));
        match done {
            Ok(vips) => shell.write().vips = vips,
            Err(why) => tell_through(motion, why),
        }
    });
}

/// The menu row that marks `address`, or takes the mark off: its words, by what `vips` holds.
pub(in crate::ui) fn menu_words(vips: &mail_client::vip::Vips, address: &str) -> &'static str {
    match mail_client::vip::toggled(vips, address) {
        Mark::Vip => copy::menus::ADD_TO_VIPS,
        Mark::NotVip => copy::menus::REMOVE_FROM_VIPS,
    }
}

/// The one glyph that means VIP, wherever a VIP is marked or listed: the mark beside a name, the
/// menu rows that add or remove one, and the VIPs place in the sidebar. Not the star, which is
/// the conversation's own.
pub(in crate::ui) const GLYPH: Icon = Icon::Sparkles;

/// The mark beside a VIP's name: a small [`GLYPH`] in the quiet ink, named VIP. Its tooltip says
/// what the mark means.
#[component]
pub(in crate::ui) fn VipMark() -> Element {
    rsx! {
        Tooltip { text: copy::aria::VIP.to_owned(),
            span { class: "vip-mark", aria_label: copy::aria::VIP, "data-vip": "true",
                Glyph { icon: GLYPH, size: IconSize::Compact }
            }
        }
    }
}
