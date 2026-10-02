//! The link pill: where a link in the reader really goes, read from the parsed blocks.

use super::hover;
use dioxus::prelude::*;
use ds::components::app::link_pill::{LinkPill as Pill, LinkTarget};
use mail_core::trust::Destination;

/// Where the link under the pointer goes, like a browser's status bar, and loud when its text
/// names somewhere else. Drawn in the reader; the link reports itself from the parsed blocks.
#[component]
pub(in crate::ui) fn LinkPill() -> Element {
    let Some(state) = hover() else {
        return rsx! {};
    };
    let Some(link) = state.link.read().clone() else {
        return rsx! {};
    };
    // quire's pill: honest or lying, keyed by where it goes so a new link plays its entrance. A
    // target with no registered domain (a `mailto:`) is shown as its scheme and its rest.
    let on_copy = EventHandler::new(|address: String| crate::ui::host::Host::copy(&address));
    match link {
        Destination::Web {
            scheme,
            sub,
            registered,
            path,
        } => rsx! {
            Pill {
                key: "{scheme}{sub}{registered}{path}",
                href: format!("{scheme}{sub}{registered}{path}"),
                oncopy: on_copy,
                target: LinkTarget::Honest { scheme_sub: format!("{scheme}{sub}"), registered, path },
            }
        },
        Destination::Lies { goes_to, claims } => rsx! {
            Pill {
                key: "{goes_to} {claims}",
                href: goes_to.clone(),
                oncopy: on_copy,
                target: LinkTarget::Lying { registered: goes_to, shown: claims },
            }
        },
        Destination::Other(href) => {
            let (scheme, rest) = href
                .split_once(':')
                .map_or(("", href.as_str()), |(scheme, rest)| (scheme, rest));
            rsx! {
                Pill {
                    key: "{href}",
                    href: href.clone(),
                    oncopy: on_copy,
                    target: LinkTarget::Honest {
                        scheme_sub: if scheme.is_empty() { String::new() } else { format!("{scheme}:") },
                        registered: rest.to_owned(),
                        path: String::new(),
                    },
                }
            }
        }
    }
}

/// A web address with its registered domain in bold and the rest dimmed: the part a lookalike
/// cannot fake, made the part that is read.
fn web(scheme: &str, sub: &str, registered: &str, path: &str) -> Element {
    rsx! {
        span { class: "dim", "{scheme}{sub}" }
        b { "{registered}" }
        span { class: "dim", "{path}" }
    }
}

/// `url` drawn the way the pill draws a link's target. Only the address is read: its text is
/// the address itself, so there is nothing for it to claim.
pub(in crate::ui) fn url_spans(url: &str) -> Element {
    match mail_core::trust::destination(url, url) {
        Destination::Web {
            scheme,
            sub,
            registered,
            path,
        } => web(&scheme, &sub, &registered, &path),
        Destination::Lies { .. } | Destination::Other(_) => {
            rsx! { span { class: "dim", "{url}" } }
        }
    }
}

/// A link in the reader was entered or left. Called from the parsed blocks.
pub(in crate::ui) fn link_over(text: &str, href: &str) {
    if let Some(mut state) = hover() {
        state
            .link
            .set(Some(mail_core::trust::destination(text, href)));
    }
}

pub(in crate::ui) fn link_out() {
    if let Some(mut state) = hover() {
        state.link.set(None);
    }
}
