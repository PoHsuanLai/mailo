//! A link's destination in Peek: what the link said, the site it goes to and its whole address,
//! read through the same honesty check as the link pill (`mail_core::trust::destination`), so a
//! link whose text names another site says so before anyone goes there. Nothing is fetched to
//! draw it: the card is the address, read, and Open Link is the only way to the page.

use dioxus::prelude::*;
use ds::components::controls::button_model::Answers;
use ds::prelude::*;
use ds::style::appearance::peek::PeekMode;
use ds::style::icon::family::PlateFamily;
use ds::style::icon::render::IconPx;
use mail_core::trust::Destination;

use crate::ui::copy;
use crate::ui::view::{LinkCard, Shell};

/// What the card says of a link: a title, the site, and a warning when the text lies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Said {
    pub title: String,
    pub site: String,
    pub warning: Option<String>,
}

/// What the card says of `card`. The title is the link's own text when it has some that is not
/// just its address; else the site.
pub(super) fn said(card: &LinkCard) -> Said {
    let (site, warning) = match mail_core::trust::destination(&card.text, &card.href) {
        Destination::Web { registered, .. } => (registered, None),
        Destination::Lies { goes_to, claims } => {
            let warning = copy::errors::link_goes_elsewhere(&goes_to, &claims);
            (goes_to, Some(warning))
        }
        Destination::Other(href) => (
            href.split_once(':')
                .map_or(href.as_str(), |(_, rest)| rest)
                .to_owned(),
            None,
        ),
    };
    let text = card.text.trim();
    let title = if text.is_empty() || text == card.href || text == site {
        site.clone()
    } else {
        text.to_owned()
    };
    Said {
        title,
        site,
        warning,
    }
}

/// Go where the link goes, as a click on it would: through the window's frames' opener, which
/// takes only an address already checked.
pub(super) fn open(card: &LinkCard) {
    if let Some(menus) = try_consume_context::<crate::ui::original::FrameMenus>() {
        menus.open(&card.href);
    }
}

/// Peek on a link. Mounted while `shell.preview` is a link.
#[component]
pub(super) fn LinkPeek(card: LinkCard, shell: Signal<Shell>) -> Element {
    let Said {
        title,
        site,
        warning,
    } = said(&card);
    let href = card.href.clone();
    rsx! {
        ds::components::app::peek::Peek {
            mode: PeekMode::Center,
            label: copy::aria::LINK_DESTINATION.to_owned(),
            onclose: move |()| super::close(shell),
            div { class: "peek-body peek-link",
                IconView {
                    source: IconSource::Glyph(Icon::Link),
                    size: IconSize::Px(IconPx(64)),
                    plate: Some(if warning.is_some() { PlateFamily::Red } else { PlateFamily::Blue }),
                }
                h2 { class: "peek-link-title", "{title}" }
                p { class: "peek-link-site", "{site}" }
                p { class: "peek-link-url", {crate::ui::hover::url_spans(&href)} }
                if let Some(warning) = warning {
                    InlineBanner { severity: Severity::Warn, text: warning }
                }
                div { class: "peek-link-actions",
                    Button {
                        label: copy::controls::COPY_LINK.to_owned(),
                        onclick: crate::ui::press::on_primary({
                            let href = href.clone();
                            move || super::pressed(|| crate::ui::host::Host::copy(&href))
                        }),
                    }
                    Button {
                        answers: Answers::Return,
                        label: copy::controls::OPEN_LINK.to_owned(),
                        title_shortcut: crate::ui::actions::open_in_app_tip(),
                        onclick: crate::ui::press::on_primary({
                            let card = card.clone();
                            move || {
                                open(&card);
                                super::close(shell);
                            }
                        }),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(text: &str, href: &str) -> LinkCard {
        LinkCard {
            text: text.to_owned(),
            href: href.to_owned(),
        }
    }

    #[test]
    fn a_link_says_its_text_and_the_site_it_goes_to() {
        let honest = said(&card("the agenda", "https://docs.example.test/agenda"));
        assert_eq!(honest.title, "the agenda");
        assert_eq!(honest.site, "example.test");
        assert_eq!(honest.warning, None);

        // A bare address is titled by its site, not repeated.
        let bare = said(&card(
            "https://docs.example.test/agenda",
            "https://docs.example.test/agenda",
        ));
        assert_eq!(bare.title, "example.test");
    }

    #[test]
    fn a_link_whose_text_names_another_site_warns() {
        let lying = said(&card(
            "https://bank.com",
            "https://bank-com.attacker.test/login",
        ));
        assert_eq!(lying.site, "attacker.test");
        assert_eq!(
            lying.warning.as_deref(),
            Some(copy::errors::link_goes_elsewhere("attacker.test", "bank.com").as_str())
        );
    }

    #[test]
    fn a_mail_link_is_its_address() {
        let mail = said(&card("", "mailto:ada@example.test"));
        assert_eq!(mail.site, "ada@example.test");
        assert_eq!(mail.title, "ada@example.test");
    }
}
