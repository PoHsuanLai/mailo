//! VIPs in the window: the mark beside a VIP's name, in a row and in the reader, and marking or
//! unmarking a sender from the reader's name menu, a row's menu or Settings.
//!
//! Who is a VIP is the client's (`mail_core::vip`); the window holds a copy in [`Shell::vips`],
//! read again on every revision as the labels are, so a mark made anywhere moves the rows, the
//! reader and the VIPs place together.

use crate::ui::copy;
use crate::ui::motion::{Follow, tell};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use ds::style::icon::render::Glyph;
use mail_core::vip::Mark;

/// Where a mark made in the reader lands: the window's shell, and its revision when it has one.
/// The reader provides it, so the sender's name below it can mark without being handed both.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Marking {
    pub shell: Signal<Shell>,
    pub revision: Option<Signal<u64>>,
}

/// Mark `address` a VIP (`name` the name the mail gave) or take the mark off, and say only a
/// failure. The shell's copy is read again at once, and the revision moved so the VIPs place and
/// every other window follow.
pub(in crate::ui) fn set(marking: Marking, address: &str, name: Option<&str>, mark: Mark) {
    let Marking {
        mut shell,
        revision,
    } = marking;
    let client = crate::ui::live::client();
    if let Err(why) = client.query().set_vip(address, name, mark) {
        tell(crate::ui::remedy::told(why), Follow::Nothing);
        return;
    }
    shell.write().vips = client.query().vips();
    if let Some(mut revision) = revision {
        revision += 1;
    }
}

/// The menu row that marks `address`, or takes the mark off: its words, by what `vips` holds.
pub(in crate::ui) fn menu_words(vips: &mail_core::vip::Vips, address: &str) -> &'static str {
    match mail_core::vip::toggled(vips, address) {
        Mark::Vip => copy::menus::ADD_TO_VIPS,
        Mark::NotVip => copy::menus::REMOVE_FROM_VIPS,
    }
}

/// The mark beside a VIP's name: a small star in the quiet ink, named VIP. Its tooltip says what
/// the star means, since a row's own star is the conversation's.
#[component]
pub(in crate::ui) fn VipMark() -> Element {
    rsx! {
        Tooltip { text: copy::aria::VIP.to_owned(),
            span { class: "vip-mark", aria_label: copy::aria::VIP, "data-vip": "true",
                Glyph { icon: Icon::Star, size: IconSize::Compact }
            }
        }
    }
}
